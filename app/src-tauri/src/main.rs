//! Model Launcher's window. The work happens in ml-core, which serves the local API this window
//! (and every other client) uses; this file only adds native bits: chat windows, connecting MCP
//! clients, and stopping models on quit.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

struct App {
    core: Arc<ml_core::Core>,
}

/// Opens a model's chat page in its own window (one chat window, reused across models).
#[tauri::command]
fn open_chat(app: tauri::AppHandle, url: String, title: String) -> Result<(), String> {
    let parsed: tauri::Url = url.parse().map_err(|e| format!("{e}"))?;
    if parsed.host_str() != Some("127.0.0.1") {
        return Err("only local chat pages can be opened".into());
    }
    if let Some(w) = app.get_webview_window("chat") {
        w.navigate(parsed).map_err(|e| e.to_string())?;
        let _ = w.set_title(&title);
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, "chat", WebviewUrl::External(parsed))
        .title(title)
        .inner_size(1040.0, 780.0)
        .min_inner_size(520.0, 420.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Opens a web link (docs, GitHub) in the default browser.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("only https links".into());
    }
    let opener = if cfg!(target_os = "macos") { "/usr/bin/open" } else { "xdg-open" };
    std::process::Command::new(opener).arg(url).status().map_err(|e| e.to_string())?;
    Ok(())
}

/// Lets the person pick model files with the system file dialog; returns their paths.
#[cfg(target_os = "linux")]
#[tauri::command]
fn pick_files() -> Vec<String> {
    // zenity (GNOME and most desktops) or kdialog (KDE)
    let tries: [(&str, &[&str]); 2] = [
        ("zenity", &["--file-selection", "--multiple", "--separator=\n", "--title=Choose model files (.gguf, .bin or .safetensors)"]),
        ("kdialog", &["--getopenfilename", ".", "--multiple", "--separate-output"]),
    ];
    for (cmd, args) in tries {
        if let Ok(o) = std::process::Command::new(cmd).args(args).output() {
            return String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.is_empty()).map(str::to_string).collect();
        }
    }
    Vec::new()
}

/// Lets the person pick model files with the standard macOS file dialog; returns their paths.
#[cfg(target_os = "macos")]
#[tauri::command]
fn pick_files() -> Vec<String> {
    let script = r#"set fs to choose file with prompt "Choose model files (.gguf, .bin or .safetensors)" with multiple selections allowed
set out to ""
repeat with f in fs
set out to out & POSIX path of f & linefeed
end repeat
return out"#;
    std::process::Command::new("/usr/bin/osascript")
        .args(["-e", script])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

/// The MCP server binary that ships inside the app.
fn mcp_path() -> PathBuf {
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_default();
    dir.join("model-launcher-mcp")
}

#[tauri::command]
fn mcp_info() -> Value {
    let path = mcp_path();
    json!({
        "path": path,
        "exists": path.is_file(),
        "config": { "mcpServers": { "model-launcher": { "command": path, "args": [] } } },
        "clients": clients().iter().map(|(id, name, file)| json!({
            "id": id, "name": name, "file": file,
            "installed": file.parent().is_some_and(|p| p.exists()),
            "connected": connected(file),
        })).collect::<Vec<_>>(),
    })
}

/// MCP clients whose config file Model Launcher can add itself to.
fn clients() -> Vec<(&'static str, &'static str, PathBuf)> {
    let home: PathBuf = std::env::var_os("HOME").map(Into::into).unwrap_or_default();
    vec![
        ("claude_desktop", "Claude", if cfg!(target_os = "macos") {
            home.join("Library/Application Support/Claude/claude_desktop_config.json")
        } else {
            home.join(".config/Claude/claude_desktop_config.json")
        }),
        ("claude_code", "Claude Code", home.join(".claude.json")),
        ("cursor", "Cursor", home.join(".cursor/mcp.json")),
        ("windsurf", "Windsurf", home.join(".codeium/windsurf/mcp_config.json")),
    ]
}

fn connected(file: &PathBuf) -> bool {
    std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .is_some_and(|v| v["mcpServers"]["model-launcher"].is_object())
}

/// Adds Model Launcher's MCP server to a client's config, keeping everything else in the file.
#[tauri::command]
fn connect_client(id: String) -> Result<String, String> {
    let (_, name, file) = clients().into_iter().find(|(c, _, _)| *c == id).ok_or("unknown app")?;
    let mut v: Value = match std::fs::read(&file) {
        Ok(b) => serde_json::from_slice(&b).map_err(|_| format!("{} isn't valid JSON, so it was left alone", file.display()))?,
        Err(_) => json!({}),
    };
    if !v.is_object() {
        return Err(format!("{} isn't a JSON object, so it was left alone", file.display()));
    }
    if !v["mcpServers"].is_object() {
        v["mcpServers"] = json!({});
    }
    v["mcpServers"]["model-launcher"] = json!({ "type": "stdio", "command": mcp_path(), "args": [] });
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())?;
    Ok(format!("Added to {name}. Restart {name} (or start a new chat) to see the tools."))
}

fn main() {
    let core = ml_core::Core::new();
    let serving = core.clone();
    let app = tauri::Builder::default()
        .manage(App { core: core.clone() })
        .invoke_handler(tauri::generate_handler![open_chat, open_url, pick_files, mcp_info, connect_client])
        .setup(move |app| {
            let stopping = serving.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = serving.serve().await {
                    eprintln!("Model Launcher's API couldn't start: {e:#}");
                }
            });
            // a terminate signal (logout, kill) skips the normal quit path, so free models here too
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tokio::signal::unix::{signal, SignalKind};
                let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else { return };
                tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
                stopping.engines.shutdown().await;
                handle.exit(0);
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Model Launcher failed to start");

    app.run(|handle, event| match event {
        // closing the main window hides it; the app (and a loaded model) keeps running in the Dock.
        // Linux has no Dock to bring it back from, so there closing it quits as usual.
        #[cfg(target_os = "macos")]
        RunEvent::WindowEvent { label, event: tauri::WindowEvent::CloseRequested { api, .. }, .. } if label == "main" => {
            api.prevent_close();
            if let Some(w) = handle.get_webview_window("main") {
                let _ = w.hide();
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            if let Some(w) = handle.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        // quitting frees every model's memory
        RunEvent::Exit => {
            let core = handle.state::<App>().core.clone();
            tauri::async_runtime::block_on(core.engines.shutdown());
        }
        _ => {}
    });
}
