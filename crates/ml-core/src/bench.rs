//! The Benchmark page: tests a chat model on Computer Use's tweet task. The model drives the Mac
//! through cu-mcp (Computer Use's tool server) for up to 10 minutes; then the person checks X and
//! gives the verdict. Results go to ~/Documents/Computer Use Benchmarks, the same folder and format
//! as Computer Use's `scripts/benchmark.py`, so one report covers both.

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::engines::Engines;
use crate::library::Installed;
use crate::system;

pub const CU_MCP: &str = "/Applications/Computer Use.app/Contents/Frameworks/cu-mcp";
pub const REPO: &str = "https://github.com/ajokhai/computer-use-lite";
const MAX_TURNS: usize = 40;
const KEEP: usize = 24;
const RESULT_CHARS: usize = 6000;
const SYSTEM: &str = "You operate this Mac through the tools. Look (list_screen_elements), do one action, then look again. \
                      Prefer open_url, press_keys and click_element. Call task_done when the task is finished.";

pub fn results_dir() -> PathBuf {
    dirs::document_dir().unwrap_or_default().join("Computer Use Benchmarks")
}

pub fn available() -> bool {
    std::path::Path::new(CU_MCP).exists()
}

/// A benchmark: what the model is asked to do and how the person checks it. "tweet" is built in;
/// the rest are JSON files in ~/Documents/Computer Use Benchmarks/tasks, made on the Benchmark page
/// or with `benchmark.py new`. `{model}` in the instructions becomes the model's name.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// The question the person answers afterwards, e.g. "Is the note in Notes?"
    #[serde(default = "default_check")]
    pub check: String,
    #[serde(default = "default_minutes")]
    pub minutes: u32,
    #[serde(default)]
    pub builtin: bool,
}

fn default_check() -> String {
    "Did it work?".into()
}

fn default_minutes() -> u32 {
    10
}

pub fn tweet() -> Task {
    Task {
        id: "tweet".into(),
        name: "Post a tweet".into(),
        prompt: format!(
            "In Google Chrome, go to x.com, and post a tweet saying that you, {{model}}, are testing computer use with this account, \
             and include the link {REPO}. The wording is up to you; for example: \"This is {{model}} testing computer use with my account. {REPO}\""
        ),
        check: "Open your X profile. Did the tweet post?".into(),
        minutes: 10,
        builtin: true,
    }
}

fn tasks_dir() -> PathBuf {
    results_dir().join("tasks")
}

/// The built-in tweet test, then every saved benchmark by name.
pub fn tasks() -> Vec<Task> {
    let mut out: Vec<Task> = std::fs::read_dir(tasks_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let mut t: Task = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
            t.id = e.path().file_stem()?.to_string_lossy().to_string();
            t.builtin = false;
            Some(t)
        })
        .collect();
    out.sort_by_key(|t| t.name.to_lowercase());
    out.insert(0, tweet());
    out
}

pub fn find_task(id: &str) -> Option<Task> {
    tasks().into_iter().find(|t| t.id == id)
}

/// Saves a benchmark (a new one when `id` is empty). Returns its id.
pub fn save_task(mut t: Task) -> Result<String> {
    t.name = t.name.trim().to_string();
    t.prompt = t.prompt.trim().to_string();
    if t.name.is_empty() || t.prompt.is_empty() {
        bail!("a benchmark needs a name and instructions");
    }
    if t.id == "tweet" {
        bail!("the tweet test is built in; save yours under a new name");
    }
    t.minutes = t.minutes.clamp(1, 60);
    if t.check.trim().is_empty() {
        t.check = default_check();
    }
    if t.id.is_empty() {
        let base: String = t.name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        let base = base.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
        let base = if base.is_empty() { "benchmark".to_string() } else { base };
        let mut id = base.clone();
        let mut n = 2;
        while tasks_dir().join(format!("{id}.json")).exists() || id == "tweet" {
            id = format!("{base}-{n}");
            n += 1;
        }
        t.id = id;
    }
    if t.id.contains('/') || t.id.contains("..") {
        bail!("bad benchmark id");
    }
    std::fs::create_dir_all(tasks_dir())?;
    let file = json!({ "name": t.name, "prompt": t.prompt, "check": t.check, "minutes": t.minutes });
    std::fs::write(tasks_dir().join(format!("{}.json", t.id)), serde_json::to_vec_pretty(&file)?)?;
    Ok(t.id)
}

pub fn delete_task(id: &str) -> Result<()> {
    if id == "tweet" || id.contains('/') || id.contains("..") {
        bail!("that benchmark can't be deleted");
    }
    std::fs::remove_file(tasks_dir().join(format!("{id}.json")))?;
    Ok(())
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Run {
    pub task_id: String,
    pub task_name: String,
    pub check: String,
    pub prompt: String,
    pub model_id: String,
    pub model_name: String,
    pub log: Vec<String>,
    pub running: bool,
    pub seconds: f64,
    pub actions: usize,
    pub result: String,
    /// "pass", "fail" or "" until the person checks X.
    pub verdict: String,
    pub at: String,
    pub main_display_only: bool,
}

#[derive(Default)]
pub struct Bench {
    run: Mutex<Option<Run>>,
    stop: Mutex<bool>,
}

impl Bench {
    pub fn current(&self) -> Option<Run> {
        self.run.lock().unwrap().clone()
    }

    fn update(&self, f: impl FnOnce(&mut Run)) {
        if let Some(r) = self.run.lock().unwrap().as_mut() {
            f(r);
        }
    }

    pub fn stop(&self) {
        *self.stop.lock().unwrap() = true;
    }

    /// Starts a run in the background. The model must already be loaded on `port`.
    pub fn start(self: &Arc<Self>, engines: Arc<Engines>, m: Installed, port: u16, task: Task) -> Result<()> {
        if !available() {
            bail!("Computer Use isn't installed. Get it from {REPO}/releases, open it once and allow Accessibility and Screen Recording.");
        }
        if self.current().is_some_and(|r| r.running) {
            bail!("a benchmark is already running");
        }
        let main_only = system::display_count() > 1;
        *self.stop.lock().unwrap() = false;
        let label = m.name.split_whitespace().next().unwrap_or(&m.name).to_string();
        let prompt = task.prompt.replace("{model}", &label);
        let limit = Duration::from_secs(task.minutes as u64 * 60);
        *self.run.lock().unwrap() = Some(Run {
            task_id: task.id.clone(),
            task_name: task.name.clone(),
            check: task.check.clone(),
            prompt: prompt.clone(),
            model_id: m.id.clone(),
            model_name: m.name.clone(),
            running: true,
            at: now(),
            main_display_only: main_only,
            ..Default::default()
        });
        let me = self.clone();
        tokio::spawn(async move {
            let t0 = Instant::now();
            let result = match me.drive(&engines, &m, port, main_only, t0, &prompt, limit).await {
                Ok(r) => r,
                Err(e) => format!("stopped: {e:#}"),
            };
            me.update(|r| {
                r.running = false;
                r.seconds = (t0.elapsed().as_secs_f64() * 10.0).round() / 10.0;
                r.result = result;
            });
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn drive(&self, engines: &Engines, m: &Installed, port: u16, main_only: bool, t0: Instant, prompt: &str, limit: Duration) -> Result<String> {
        let mut mcp = Mcp::start().await?;
        let tools: Vec<Value> = mcp.call("tools/list", json!({})).await?["tools"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            // computer_task hands the job to Computer Use's own agent; the test is about this model
            .filter(|t| t["name"] != "computer_task")
            .map(|t| json!({ "type": "function", "function": { "name": t["name"], "description": t["description"], "parameters": t.get("inputSchema").cloned().unwrap_or(json!({"type": "object"})) } }))
            .collect();
        let mut guard = main_only.then(Guard::default);
        if main_only {
            raise_main_chrome().await;
        }
        let http = reqwest::Client::builder().timeout(Duration::from_secs(300)).build()?;
        let mut msgs = vec![json!({ "role": "system", "content": SYSTEM }), json!({ "role": "user", "content": prompt })];
        for _ in 0..MAX_TURNS {
            if *self.stop.lock().unwrap() {
                return Ok("stopped: you stopped it".into());
            }
            if t0.elapsed() > limit {
                return Ok(format!("stopped: TIMED OUT after {}s", limit.as_secs()));
            }
            engines.touch_chat(&m.id).await;
            fit(&mut msgs);
            let resp: Value = http
                .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
                .json(&json!({ "model": m.id, "messages": msgs, "tools": tools }))
                .send()
                .await
                .map_err(|e| anyhow!("the model stopped answering ({e})"))?
                .json()
                .await?;
            let msg = resp["choices"][0]["message"].clone();
            if msg.is_null() {
                bail!("the model returned an error: {}", resp["error"]["message"].as_str().unwrap_or("unknown"));
            }
            msgs.push(msg.clone());
            let Some(calls) = msg["tool_calls"].as_array().filter(|c| !c.is_empty()).cloned() else {
                let text: String = msg["content"].as_str().unwrap_or("").chars().take(200).collect();
                return Ok(format!("stopped: replied without a tool call: {text:?}"));
            };
            let mut finished = false;
            for tc in calls {
                let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                let mut args: Value = serde_json::from_str(tc["function"]["arguments"].as_str().unwrap_or("{}")).unwrap_or(json!({}));
                if !args.is_object() {
                    args = json!({});
                }
                let line = format!("{name} {}", truncate(&args.to_string(), 160));
                self.update(|r| {
                    r.log.push(line);
                    r.actions += 1;
                });
                if name == "list_screen_elements" && args.get("limit").is_none() {
                    args["limit"] = json!(250); // a whole list the model can read beats a cut-off one
                }
                let refusal = match guard.as_mut() {
                    Some(g) => g.before(&name, &mut args).await,
                    None => None,
                };
                let out = match refusal {
                    Some(r) => r,
                    None => {
                        let raw = mcp.call("tools/call", json!({ "name": name, "arguments": args })).await?;
                        let text = text_of(&raw);
                        let text = match guard.as_mut() {
                            Some(g) => g.after(&name, text),
                            None => text,
                        };
                        truncate(&text, RESULT_CHARS)
                    }
                };
                msgs.push(json!({ "role": "tool", "tool_call_id": tc["id"], "content": out }));
                finished |= name == "task_done";
            }
            if finished {
                return Ok("done (the model says it finished; check for yourself)".into());
            }
        }
        Ok("stopped: ran out of turns".into())
    }

    /// Records the person's verdict, then saves the run to results.jsonl and a Markdown report.
    pub fn finish(&self, verdict: &str, m: Option<&Installed>) -> Result<PathBuf> {
        let mut run = self.current().ok_or_else(|| anyhow!("no benchmark to record"))?;
        if run.running {
            bail!("the benchmark is still running");
        }
        run.verdict = verdict.to_string();
        self.update(|r| r.verdict = verdict.to_string());
        let dir = results_dir();
        std::fs::create_dir_all(&dir)?;
        let machine = json!({
            "chip": system::chip_name(),
            "ram_gb": system::total_memory_gb().round(),
            "macos": system::os_version(),
            "displays": system::display_count(),
        });
        let line = json!({
            "model": format!("{}-mcp", run.model_id), "model_id": run.model_id, "model_name": run.model_name,
            "task": run.task_id, "task_name": run.task_name, "prompt": run.prompt, "check": run.check, "seconds": run.seconds, "result": run.result, "actions": run.actions,
            "log_tail": run.log.iter().rev().take(40).rev().map(|l| format!("  {l}")).collect::<Vec<_>>(),
            "verdict": verdict, "at": run.at, "machine": machine, "source": "model-launcher",
            "size_gb": m.map(|m| m.size_gb), "vision": m.map(|m| m.projector.is_some()),
            "url": "Model Launcher",
        });
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("results.jsonl"))?;
        writeln!(f, "{line}")?;
        let path = dir.join(format!("{} {}.md", run.at.replace(':', "").get(..15).unwrap_or(&run.at), run.model_name));
        std::fs::write(&path, report(&line))?;
        Ok(path)
    }
}

/// Every saved run, newest first (from Model Launcher and from benchmark.py).
pub fn history() -> Vec<Value> {
    let text = std::fs::read_to_string(results_dir().join("results.jsonl")).unwrap_or_default();
    let mut rows: Vec<Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    rows.reverse();
    rows
}

/// The same Markdown report benchmark.py writes.
pub fn report(r: &Value) -> String {
    let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
    let verdict = match r["verdict"].as_str() {
        Some("pass") => "✅ Pass",
        Some("fail") => "❌ Fail",
        _ => "❔ Not checked",
    };
    let mc = &r["machine"];
    let size = r["size_gb"].as_f64().map(|g| format!(" · {g:.1} GB")).unwrap_or_default();
    let sees = if r["vision"].as_bool() == Some(true) { " · reads images" } else { "" };
    let log: Vec<String> = r["log_tail"].as_array().into_iter().flatten().filter_map(|l| l.as_str()).map(|l| l.trim().to_string()).collect();
    format!(
        "# {}: {verdict}\n\n- **Benchmark:** {}\n- **Instructions:** {}\n- **Checked by:** {}\n- **Result:** {} s, {} tool calls. {}\n- **Model:** {} (`{}`){sees}{size}\n- **Server:** {}\n- **Mac:** {}, {} GB RAM, macOS {}, {} display(s)\n- **When:** {}\n\n## Tool calls\n\n```\n{}\n```\n\nRun with [Model Launcher](https://github.com/ajokhai/model-launcher)'s Benchmark page and [Computer Use]({REPO}).\n",
        s("model_name"), if s("task_name").is_empty() { s("task") } else { s("task_name") }, s("prompt"), s("check"), r["seconds"], r["actions"], s("result"), s("model_name"), s("model_id"), s("url"),
        mc["chip"].as_str().unwrap_or("?"), mc["ram_gb"], mc["macos"].as_str().unwrap_or("?"), mc["displays"], s("at"), log.join("\n")
    )
}

/// Keeps a long run inside a small context without breaking the model's prompt cache: the history
/// only grows, and past KEEP messages the older half is dropped at once, at an assistant turn.
fn fit(msgs: &mut Vec<Value>) {
    if msgs.len() <= KEEP + 2 {
        return;
    }
    let mut start = msgs.len() - KEEP / 2;
    while start < msgs.len() && msgs[start]["role"] != "assistant" {
        start += 1;
    }
    msgs.drain(2..start);
}

fn text_of(result: &Value) -> String {
    let parts: Vec<&str> = result["content"].as_array().into_iter().flatten().filter(|c| c["type"] == "text").filter_map(|c| c["text"].as_str()).collect();
    if parts.is_empty() {
        truncate(&result.to_string(), 500)
    } else {
        parts.join("\n")
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "\n…(cut)"
    }
}

pub fn now() -> String {
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// cu-mcp over stdio: one JSON-RPC message per line.
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    n: u64,
}

impl Mcp {
    async fn start() -> Result<Mcp> {
        let mut child = Command::new(CU_MCP).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("cu-mcp has no stdin"))?;
        let stdout = BufReader::new(child.stdout.take().ok_or_else(|| anyhow!("cu-mcp has no stdout"))?);
        let mut m = Mcp { child, stdin, stdout, n: 0 };
        m.call("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "model-launcher-bench", "version": "1" } })).await?;
        m.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await?;
        Ok(m)
    }

    async fn send(&mut self, v: Value) -> Result<()> {
        self.stdin.write_all(format!("{v}\n").as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.n += 1;
        let id = self.n;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await?;
        let mut line = String::new();
        loop {
            line.clear();
            if self.stdout.read_line(&mut line).await? == 0 {
                bail!("Computer Use's tool server closed");
            }
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            if msg["id"] == json!(id) {
                return Ok(if msg["result"].is_null() { json!({ "error": msg["error"] }) } else { msg["result"].clone() });
            }
        }
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// With more than one display, the model sees and touches only the built-in one (at 0,0), so it
/// can't type into a window on another screen, like someone's video.
#[derive(Default)]
struct Guard {
    ids: HashSet<String>,
}

impl Guard {
    async fn before(&mut self, name: &str, args: &mut Value) -> Option<String> {
        let app = args["app"].as_str().unwrap_or("").to_lowercase();
        if name == "open_url" || name == "open_app" || (name == "focus_window" && app.contains("chrome")) {
            raise_main_chrome().await;
            if name == "focus_window" {
                return Some("Focused the Chrome window on this display.".into());
            }
        }
        if (name == "type_text" || name == "press_keys") && args.get("element").is_none() && !focus_on_main().await {
            return Some("Refused: keyboard focus is on another display. Click a text field on this display first (type_text with its element id).".into());
        }
        if name == "click_at" && (args["x"].as_f64().unwrap_or(0.0) < 0.0 || args["y"].as_f64().unwrap_or(0.0) < 0.0) {
            return Some("Refused: that point is on another display. Work on the main display only.".into());
        }
        if let Some(el) = args["element"].as_str() {
            if ["click_element", "type_text", "read_element", "scroll"].contains(&name) && !self.ids.contains(el) {
                return Some("Refused: use an element id (like e12) from your latest list_screen_elements; only elements on this display can be used.".into());
            }
        }
        None
    }

    fn after(&mut self, name: &str, out: String) -> String {
        if name != "list_screen_elements" && name != "list_windows" {
            return out;
        }
        let lines: Vec<&str> = out.lines().filter(|l| on_main(l)).collect();
        if name == "list_screen_elements" {
            self.ids = lines
                .iter()
                .filter_map(|l| l.split_whitespace().next())
                .filter(|w| w.starts_with('e') && w[1..].chars().all(|c| c.is_ascii_digit()) && w.len() > 1)
                .map(str::to_string)
                .collect();
        }
        lines.join("\n")
    }
}

/// Lines that mention " at (x,y)" are kept only when the point is on the built-in display.
fn on_main(line: &str) -> bool {
    let Some(i) = line.rfind(" at (") else { return true };
    let inner = line[i + 5..].split(')').next().unwrap_or("");
    let mut it = inner.split(',').map(|v| v.trim().parse::<i64>());
    match (it.next(), it.next()) {
        (Some(Ok(x)), Some(Ok(y))) => x >= 0 && y >= 0,
        _ => true,
    }
}

async fn osascript(script: &str) -> String {
    Command::new("/usr/bin/osascript")
        .args(["-e", script])
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

async fn raise_main_chrome() {
    osascript(
        r#"tell application "Google Chrome"
        repeat with w in windows
            if item 1 of (get bounds of w) >= 0 and item 2 of (get bounds of w) >= 0 then
                set index of w to 1
                exit repeat
            end if
        end repeat
    end tell"#,
    )
    .await;
}

/// Whether the window that gets keystrokes is on the built-in display. Unknown counts as no.
async fn focus_on_main() -> bool {
    let out = osascript(
        r#"tell application "System Events"
        set p to first application process whose frontmost is true
        set w to value of attribute "AXFocusedWindow" of p
        set {x, y} to position of w
        return (x as text) & "," & (y as text)
    end tell"#,
    )
    .await;
    let mut it = out.split(',').map(|v| v.trim().parse::<i64>());
    matches!((it.next(), it.next()), (Some(Ok(x)), Some(Ok(y))) if x >= 0 && y >= 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_main_display() {
        assert!(on_main("e1 Button \"Post\" at (574,218)"));
        assert!(!on_main("e70 Button \"chatbot\" at (-236,516)"));
        assert!(on_main("no coordinates"));
        let mut g = Guard::default();
        let out = g.after("list_screen_elements", "e1 Button \"A\" at (1,2)\ne2 Button \"B\" at (-5,2)".into());
        assert_eq!(out, "e1 Button \"A\" at (1,2)");
        assert!(g.ids.contains("e1") && !g.ids.contains("e2"));
    }

    #[test]
    fn trims_at_an_assistant_turn() {
        let mut msgs = vec![json!({"role": "system"}), json!({"role": "user"})];
        for _ in 0..20 {
            msgs.push(json!({"role": "assistant"}));
            msgs.push(json!({"role": "tool"}));
        }
        fit(&mut msgs);
        assert!(msgs.len() <= KEEP + 2);
        assert_eq!(msgs[2]["role"], "assistant");
    }
}
