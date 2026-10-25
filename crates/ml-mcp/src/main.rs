//! model-launcher-mcp: an MCP server (stdio) that gives any MCP client (Claude, Cursor, VS Code…)
//! the models running in Model Launcher: chat, transcribe, speak and generate_image, plus listing,
//! loading and unloading models. It talks to the app's local API and opens the app if it isn't
//! running. Its only dependency is serde_json; HTTP to localhost is written by hand.

use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn port() -> u16 {
    let home: std::path::PathBuf = std::env::var_os("HOME").map(Into::into).unwrap_or_default();
    std::fs::read(home.join("Library/Application Support/Model Launcher/settings.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["api_port"].as_u64())
        .map(|p| p as u16)
        .unwrap_or(11500)
}

/// One HTTP/1.1 request to the local API. Returns (status, body).
fn http(method: &str, path: &str, ctype: &str, body: &[u8], timeout: Duration) -> Result<(u16, Vec<u8>), String> {
    let mut s = TcpStream::connect(("127.0.0.1", port())).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(timeout)).ok();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).and_then(|_| s.write_all(body)).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("bad response")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_lowercase();
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let mut body = raw[split + 4..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        body = dechunk(&body);
    }
    Ok((status, body))
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(nl) = b.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&b[..nl]).trim(), 16).unwrap_or(0);
        if size == 0 || nl + 2 + size > b.len() {
            break;
        }
        out.extend_from_slice(&b[nl + 2..nl + 2 + size]);
        b = &b[(nl + 4 + size).min(b.len())..];
    }
    out
}

/// Calls the API, opening Model Launcher first if it isn't running. Returns the raw body.
fn api_raw(method: &str, path: &str, ctype: &str, body: &[u8], timeout: Duration) -> Result<Vec<u8>, String> {
    let mut res = http(method, path, ctype, body, timeout);
    if res.is_err() {
        let _ = std::process::Command::new("/usr/bin/open").args(["-g", "-b", "com.ajokhai.modellauncher"]).status();
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(500));
            res = http(method, path, ctype, body, timeout);
            if res.is_ok() {
                break;
            }
        }
    }
    let (status, bytes) = res.map_err(|_| "Model Launcher isn't running and couldn't be opened".to_string())?;
    if status >= 400 {
        let v: Value = serde_json::from_slice(&bytes).unwrap_or_default();
        return Err(v["error"]["message"].as_str().unwrap_or("request failed").to_string());
    }
    Ok(bytes)
}

fn api(method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value, String> {
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let bytes = api_raw(method, path, "application/json", payload.as_bytes(), timeout)?;
    Ok(serde_json::from_slice(&bytes).unwrap_or_default())
}

fn tools() -> Value {
    json!([
        { "name": "list_models", "description": "Models installed in Model Launcher with their kind (chat, transcribe, speak, image), size and fit for this Mac's memory budget, and which chat model is loaded.",
          "inputSchema": { "type": "object", "properties": {} } },
        { "name": "chat", "description": "Ask a local chat model. Loads (hot-swaps) the model if needed; omit model to use the loaded one.",
          "inputSchema": { "type": "object", "properties": {
              "prompt": { "type": "string" },
              "model": { "type": "string", "description": "Model id from list_models" },
              "system": { "type": "string" },
              "max_tokens": { "type": "integer" } }, "required": ["prompt"] } },
        { "name": "transcribe", "description": "Speech to text with a local Whisper model. Takes wav, mp3, m4a, aiff and other audio files.",
          "inputSchema": { "type": "object", "properties": {
              "path": { "type": "string", "description": "Absolute path to the audio file" },
              "model": { "type": "string" } }, "required": ["path"] } },
        { "name": "speak", "description": "Text to speech with a local model. Returns the path of the WAV it wrote; play=true also plays it.",
          "inputSchema": { "type": "object", "properties": {
              "text": { "type": "string" }, "model": { "type": "string" }, "play": { "type": "boolean" } }, "required": ["text"] } },
        { "name": "generate_image", "description": "Make an image with a local model. Returns the PNG's path.",
          "inputSchema": { "type": "object", "properties": {
              "prompt": { "type": "string" }, "model": { "type": "string" },
              "size": { "type": "string", "description": "WIDTHxHEIGHT, e.g. 512x512" },
              "steps": { "type": "integer" } }, "required": ["prompt"] } },
        { "name": "load_model", "description": "Load a chat model now (unloading the current one) so the next chat is instant.",
          "inputSchema": { "type": "object", "properties": { "model": { "type": "string" } }, "required": ["model"] } },
        { "name": "unload_model", "description": "Unload the chat model and free its memory.",
          "inputSchema": { "type": "object", "properties": {} } }
    ])
}

fn call(name: &str, a: &Value) -> Result<String, String> {
    let long = Duration::from_secs(600);
    let s = |k: &str| a[k].as_str().map(str::to_string);
    match name {
        "list_models" => {
            let lib = api("GET", "/api/library", None, Duration::from_secs(10))?;
            let st = api("GET", "/api/status", None, Duration::from_secs(10))?;
            let mut lines: Vec<String> = lib
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| {
                    format!(
                        "{} ({}): {}, {:.1} GB, fit {}",
                        m["id"].as_str().unwrap_or(""),
                        m["kind"].as_str().unwrap_or(""),
                        m["name"].as_str().unwrap_or(""),
                        m["size_gb"].as_f64().unwrap_or(0.0),
                        m["fit"].as_str().unwrap_or("")
                    )
                })
                .collect();
            if lines.is_empty() {
                lines.push("No models installed. Open Model Launcher › Discover to get some.".into());
            }
            lines.push(match st["running"]["name"].as_str() {
                Some(n) => format!("Loaded chat model: {n}"),
                None => "No chat model loaded.".into(),
            });
            lines.push(format!("Memory budget: {:.0} GB", st["settings"]["memory_budget_gb"].as_f64().unwrap_or(0.0)));
            Ok(lines.join("\n"))
        }
        "chat" => {
            let mut messages = Vec::new();
            if let Some(sys) = s("system") {
                messages.push(json!({ "role": "system", "content": sys }));
            }
            messages.push(json!({ "role": "user", "content": s("prompt").unwrap_or_default() }));
            let mut body = json!({ "messages": messages, "stream": false });
            if let Some(m) = s("model") {
                body["model"] = json!(m);
            }
            if let Some(n) = a["max_tokens"].as_u64() {
                body["max_tokens"] = json!(n);
            }
            let r = api("POST", "/v1/chat/completions", Some(&body), long)?;
            Ok(r["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string())
        }
        "transcribe" => {
            let path = s("path").ok_or("path is required")?;
            let audio = std::fs::read(&path).map_err(|e| format!("can't read {path}: {e}"))?;
            let fname = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or("audio".into());
            let boundary = "----modellauncher7d1b";
            let mut body = Vec::new();
            if let Some(m) = s("model") {
                body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n{m}\r\n").as_bytes());
            }
            body.extend(
                format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{fname}\"\r\nContent-Type: application/octet-stream\r\n\r\n")
                    .as_bytes(),
            );
            body.extend(&audio);
            body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
            let ctype = format!("multipart/form-data; boundary={boundary}");
            let bytes = api_raw("POST", "/v1/audio/transcriptions", &ctype, &body, long)?;
            let v: Value = serde_json::from_slice(&bytes).unwrap_or_default();
            Ok(v["text"].as_str().unwrap_or("").to_string())
        }
        "speak" => {
            let mut body = json!({ "input": s("text").unwrap_or_default() });
            if let Some(m) = s("model") {
                body["model"] = json!(m);
            }
            let wav = api_raw("POST", "/v1/audio/speech", "application/json", body.to_string().as_bytes(), long)?;
            let dir = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default()
                .join("Library/Application Support/Model Launcher/outputs");
            let _ = std::fs::create_dir_all(&dir);
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let out = dir.join(format!("speech-{stamp}.wav"));
            std::fs::write(&out, &wav).map_err(|e| e.to_string())?;
            if a["play"].as_bool().unwrap_or(false) {
                let _ = std::process::Command::new("/usr/bin/afplay").arg(&out).status();
            }
            Ok(format!("Wrote {}", out.display()))
        }
        "generate_image" => {
            let mut body = json!({ "prompt": s("prompt").unwrap_or_default() });
            for k in ["model", "size"] {
                if let Some(v) = s(k) {
                    body[k] = json!(v);
                }
            }
            if let Some(n) = a["steps"].as_u64() {
                body["steps"] = json!(n);
            }
            let r = api("POST", "/v1/images/generations", Some(&body), long)?;
            Ok(format!("Saved {}", r["data"][0]["path"].as_str().unwrap_or("?")))
        }
        "load_model" => {
            let r = api("POST", "/api/launch", Some(&json!({ "id": s("model").unwrap_or_default() })), long)?;
            Ok(format!("{} is loaded. Its chat page: {}", r["name"].as_str().unwrap_or(""), r["url"].as_str().unwrap_or("")))
        }
        "unload_model" => {
            api("POST", "/api/stop", None, Duration::from_secs(30))?;
            Ok("Unloaded.".into())
        }
        _ => Err(format!("unknown tool {name}")),
    }
}

fn main() {
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!(
            "model-launcher-mcp {VERSION}\nMCP server (stdio) for Model Launcher. Add it to an MCP client as a stdio server whose command is this file's path.\nTools: list_models, chat, transcribe, speak, generate_image, load_model, unload_model."
        );
        return;
    }
    let mut out = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue }; // notifications need no reply
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => Ok(json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "model-launcher", "version": VERSION },
                "instructions": "Open-source models running locally on this Mac through Model Launcher. Call list_models first."
            })),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => {
                let p = &msg["params"];
                Ok(match call(p["name"].as_str().unwrap_or(""), &p["arguments"]) {
                    Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
                    Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
                })
            }
            "ping" => Ok(json!({})),
            m => Err(json!({ "code": -32601, "message": format!("method not found: {m}") })),
        };
        let reply = match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
        };
        let _ = writeln!(out, "{reply}");
        let _ = out.flush();
    }
}
