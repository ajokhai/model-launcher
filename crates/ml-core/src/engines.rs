//! Runs models and keeps them inside the user's limits.
//!
//! One chat model at a time runs as a llama-server; asking for another swaps it (hot swap). Voice and
//! image models run one job at a time as short-lived processes. Every process runs at the user's
//! thread count and priority, and a watchdog stops anything that grows past the memory budget or
//! sits idle too long.

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::catalog::Kind;
use crate::library::Installed;
use crate::settings::{logs_dir, support_dir, Settings};
use crate::system;

/// The engine programs Model Launcher drives.
#[derive(Clone, Copy, Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    LlamaServer,
    LlamaTts,
    WhisperCli,
    SdCli,
}

impl Engine {
    pub const ALL: [Engine; 4] = [Engine::LlamaServer, Engine::LlamaTts, Engine::WhisperCli, Engine::SdCli];

    pub fn binary(self) -> &'static str {
        match self {
            Engine::LlamaServer => "llama-server",
            Engine::LlamaTts => "llama-tts",
            Engine::WhisperCli => "whisper-cli",
            Engine::SdCli => "sd-cli",
        }
    }

    pub fn for_kind(kind: Kind) -> Engine {
        match kind {
            Kind::Chat => Engine::LlamaServer,
            Kind::Speak => Engine::LlamaTts,
            Kind::Transcribe => Engine::WhisperCli,
            Kind::Image => Engine::SdCli,
        }
    }

    /// Where it's found: Model Launcher's own engines folder first, then Homebrew.
    pub fn path(self) -> Option<PathBuf> {
        [engines_dir(), "/opt/homebrew/bin".into(), "/usr/local/bin".into()]
            .into_iter()
            .map(|d| d.join(self.binary()))
            .find(|p| p.is_file())
    }
}

pub fn engines_dir() -> PathBuf {
    support_dir().join("engines")
}

/// Memory a model needs while running, in GB: its files plus working memory. Deliberately a little
/// generous; the watchdog measures the real number.
pub fn estimate_gb(m: &Installed, s: &Settings) -> f64 {
    match m.kind {
        Kind::Chat => {
            let weights = system_size(&m.weights);
            // q8 KV cache: about 75 KB per token for an 8B model, scaled by model size
            let kv = s.context_size as f64 * 7.5e-5 * (weights / 5.0).clamp(0.2, 3.0);
            m.size_gb + kv + 0.4
        }
        Kind::Image => m.size_gb * 1.3 + 1.2,
        Kind::Speak => m.size_gb + 0.6,
        Kind::Transcribe => m.size_gb * 1.5 + 0.3,
    }
}

fn system_size(p: &Path) -> f64 {
    std::fs::metadata(p).map(|m| m.len() as f64 / 1024f64.powi(3)).unwrap_or(0.0)
}

#[derive(Clone, Debug, Serialize)]
pub struct Running {
    pub id: String,
    pub name: String,
    pub port: u16,
    pub pid: u32,
    pub memory_gb: f64,
    pub idle_secs: u64,
    pub ready: bool,
    pub vision: bool,
}

struct ChatServer {
    model: Installed,
    child: Child,
    port: u16,
    pid: u32,
    last_used: Instant,
    ready: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub label: String,
    pub pid: u32,
    pub memory_gb: f64,
}

pub struct Engines {
    settings: Arc<RwLock<Settings>>,
    chat: Mutex<Option<ChatServer>>,
    /// Voice and image jobs run one at a time.
    jobs: Semaphore,
    job: StdMutex<Option<Job>>,
    events: StdMutex<VecDeque<String>>,
    http: reqwest::Client,
}

impl Engines {
    pub fn new(settings: Arc<RwLock<Settings>>) -> Arc<Engines> {
        let _ = std::fs::create_dir_all(logs_dir());
        Arc::new(Engines {
            settings,
            chat: Mutex::new(None),
            jobs: Semaphore::new(1),
            job: StdMutex::new(None),
            events: StdMutex::new(VecDeque::new()),
            http: reqwest::Client::new(),
        })
    }

    pub fn note(&self, msg: impl Into<String>) {
        let mut e = self.events.lock().unwrap();
        let stamp = chrono_like_now();
        e.push_front(format!("{stamp}  {}", msg.into()));
        e.truncate(50);
    }

    pub fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().iter().cloned().collect()
    }

    pub async fn running(&self) -> Option<Running> {
        let c = self.chat.lock().await;
        c.as_ref().map(|c| Running {
            id: c.model.id.clone(),
            name: c.model.name.clone(),
            port: c.port,
            pid: c.pid,
            memory_gb: system::process_memory_gb(c.pid).unwrap_or(0.0),
            idle_secs: c.last_used.elapsed().as_secs(),
            ready: c.ready,
            vision: c.model.projector.is_some(),
        })
    }

    pub fn current_job(&self) -> Option<Job> {
        let mut j = self.job.lock().unwrap().clone();
        if let Some(j) = j.as_mut() {
            j.memory_gb = system::process_memory_gb(j.pid).unwrap_or(0.0);
        }
        j
    }

    /// Memory the running models use right now, in GB.
    pub async fn used_gb(&self) -> f64 {
        let chat = self.running().await.map(|r| r.memory_gb).unwrap_or(0.0);
        chat + self.current_job().map(|j| j.memory_gb).unwrap_or(0.0)
    }

    /// Makes sure `m` is the loaded chat model and returns its port. Swaps out whatever was loaded.
    pub async fn ensure_chat(&self, m: &Installed) -> Result<u16> {
        if m.kind != Kind::Chat {
            bail!("{} isn't a chat model", m.name);
        }
        let mut guard = self.chat.lock().await;
        if let Some(c) = guard.as_mut() {
            if c.model.id == m.id && c.child.try_wait().ok().flatten().is_none() {
                c.last_used = Instant::now();
                let port = c.port;
                let ready = c.ready;
                drop(guard);
                if !ready {
                    self.wait_ready(port).await?;
                }
                return Ok(port);
            }
        }
        if let Some(old) = guard.take() {
            self.note(format!("Swapped out {} to load {}", old.model.name, m.name));
            stop_child(old.child).await;
        }

        let s = self.settings.read().await.clone();
        let need = estimate_gb(m, &s);
        if need > s.memory_budget_gb {
            bail!(
                "{} needs about {:.1} GB, more than your {:.0} GB memory budget. Raise the budget in Settings or pick a smaller model.",
                m.name, need, s.memory_budget_gb
            );
        }
        let free = system::available_memory_gb() + self.current_job().map(|j| j.memory_gb).unwrap_or(0.0);
        if need > free + 1.0 {
            bail!(
                "{} needs about {:.1} GB but only {:.1} GB is free right now. Close some apps and try again.",
                m.name, need, free
            );
        }

        let bin = Engine::LlamaServer.path().ok_or_else(|| anyhow!("llama.cpp isn't installed yet (Settings › Engines)"))?;
        let port = free_port()?;
        let mut cmd = Command::new(bin);
        cmd.arg("-m").arg(&m.weights);
        if let Some(p) = &m.projector {
            cmd.arg("--mmproj").arg(p);
            if !s.use_gpu {
                cmd.arg("--no-mmproj-offload");
            }
        }
        cmd.args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .args(["-c", &s.context_size.to_string()])
            .args(["-t", &s.effective_threads().to_string()])
            .args(["-ngl", if s.use_gpu { "999" } else { "0" }])
            // flash attention plus an 8-bit KV cache halves the memory a long chat takes
            .args(["-fa", "on", "-ctk", "q8_0", "-ctv", "q8_0"])
            .args(["--jinja", "--parallel", "1"]);
        let child = spawn_logged(cmd, "chat")?;
        let pid = child.id().unwrap_or(0);
        let _ = std::fs::write(pid_file(), pid.to_string());
        if s.low_priority {
            system::make_low_priority(pid);
        }
        self.note(format!("Loading {} ({:.1} GB expected)", m.name, need));
        *guard = Some(ChatServer { model: m.clone(), child, port, pid, last_used: Instant::now(), ready: false });
        drop(guard);

        if let Err(e) = self.wait_ready(port).await {
            self.stop_chat("it didn't start").await;
            return Err(e);
        }
        if let Some(c) = self.chat.lock().await.as_mut() {
            c.ready = true;
        }
        self.note(format!("{} is ready", m.name));
        Ok(port)
    }

    async fn wait_ready(&self, port: u16) -> Result<()> {
        let url = format!("http://127.0.0.1:{port}/health");
        for _ in 0..600 {
            if let Ok(r) = self.http.get(&url).timeout(Duration::from_secs(1)).send().await {
                if r.status().is_success() {
                    return Ok(());
                }
            }
            {
                let mut c = self.chat.lock().await;
                match c.as_mut() {
                    Some(c) if c.port == port => {
                        if let Ok(Some(status)) = c.child.try_wait() {
                            bail!("the model server stopped while loading ({status}); see {}", logs_dir().join("chat.log").display());
                        }
                    }
                    _ => bail!("the model was unloaded while starting"),
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        bail!("the model took more than two minutes to load")
    }

    pub async fn touch_chat(&self) {
        if let Some(c) = self.chat.lock().await.as_mut() {
            c.last_used = Instant::now();
        }
    }

    pub async fn stop_chat(&self, why: &str) {
        if let Some(c) = self.chat.lock().await.take() {
            let _ = std::fs::remove_file(pid_file());
            self.note(format!("Unloaded {}: {why}", c.model.name));
            stop_child(c.child).await;
        }
    }

    /// Runs a voice or image job to completion, inside the memory budget. A loaded chat model that
    /// would push the total over budget is unloaded first (it reloads on the next chat).
    pub async fn run_job(&self, m: &Installed, label: &str, mut cmd: Command) -> Result<()> {
        let _permit = self.jobs.acquire().await?;
        let s = self.settings.read().await.clone();
        let need = estimate_gb(m, &s);
        if need > s.memory_budget_gb {
            bail!("{} needs about {:.1} GB, more than your {:.0} GB memory budget.", m.name, need, s.memory_budget_gb);
        }
        let chat_gb = self.running().await.map(|r| r.memory_gb).unwrap_or(0.0);
        if chat_gb + need > s.memory_budget_gb {
            self.stop_chat(&format!("made room for {}", m.name)).await;
        }
        cmd.stdin(Stdio::null());
        let mut child = spawn_logged(cmd, label)?;
        let pid = child.id().unwrap_or(0);
        if s.low_priority {
            system::make_low_priority(pid);
        }
        *self.job.lock().unwrap() = Some(Job { label: format!("{}: {label}", m.name), pid, memory_gb: 0.0 });
        let status = child.wait().await;
        *self.job.lock().unwrap() = None;
        let status = status?;
        if !status.success() {
            bail!("{} failed ({status}); see {}", m.name, logs_dir().join(format!("{label}.log")).display());
        }
        Ok(())
    }

    /// Every two seconds: stop whatever is over the memory budget, and unload an idle chat model.
    pub async fn watchdog(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let s = self.settings.read().await.clone();
            let job = self.current_job();
            let job_gb = job.as_ref().map(|j| j.memory_gb).unwrap_or(0.0);
            let running = self.running().await;
            let chat_gb = running.as_ref().map(|r| r.memory_gb).unwrap_or(0.0);
            // a little grace, so a brief spike while loading doesn't kill a model that fits
            let limit = s.memory_budget_gb * 1.1;
            if chat_gb + job_gb > limit {
                if job_gb > chat_gb {
                    if let Some(j) = job {
                        unsafe { libc::kill(j.pid as i32, libc::SIGTERM) };
                        self.note(format!("Stopped {}: it used {:.1} GB, over your {:.0} GB budget", j.label, job_gb, s.memory_budget_gb));
                    }
                } else {
                    self.stop_chat(&format!("it used {:.1} GB, over your {:.0} GB budget", chat_gb, s.memory_budget_gb)).await;
                }
                continue;
            }
            if let Some(r) = running {
                if s.idle_unload_minutes > 0 && r.ready && r.idle_secs >= s.idle_unload_minutes as u64 * 60 {
                    self.stop_chat(&format!("idle for {} minutes", s.idle_unload_minutes)).await;
                }
            }
            // a server that died on its own
            let mut c = self.chat.lock().await;
            if let Some(cs) = c.as_mut() {
                if let Ok(Some(st)) = cs.child.try_wait() {
                    let name = cs.model.name.clone();
                    *c = None;
                    drop(c);
                    self.note(format!("{name} stopped on its own ({st})"));
                }
            }
        }
    }

    pub async fn shutdown(&self) {
        self.stop_chat("Model Launcher quit").await;
        if let Some(j) = self.job.lock().unwrap().take() {
            unsafe { libc::kill(j.pid as i32, libc::SIGTERM) };
        }
    }

    pub fn settings(&self) -> &Arc<RwLock<Settings>> {
        &self.settings
    }
}

async fn stop_child(mut child: Child) {
    if let Some(pid) = child.id() {
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }
    if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
        let _ = child.kill().await;
    }
}

fn pid_file() -> PathBuf {
    support_dir().join("chat.pid")
}

/// Stops a model server left behind by a run that crashed or was killed, so its memory comes back.
pub fn reap_orphan() -> Option<u32> {
    let pid: i32 = std::fs::read_to_string(pid_file()).ok()?.trim().parse().ok()?;
    let _ = std::fs::remove_file(pid_file());
    let mut buf = [0u8; 4096];
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32) };
    let path = String::from_utf8_lossy(&buf[..n.max(0) as usize]).to_string();
    if n > 0 && path.ends_with("/llama-server") {
        unsafe { libc::kill(pid, libc::SIGTERM) };
        return Some(pid as u32);
    }
    None
}

fn spawn_logged(mut cmd: Command, label: &str) -> Result<Child> {
    let log = std::fs::File::create(logs_dir().join(format!("{label}.log"))).context("can't write the log")?;
    cmd.stdout(Stdio::from(log.try_clone()?)).stderr(Stdio::from(log)).kill_on_drop(true);
    cmd.spawn().context("couldn't start the engine")
}

fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

fn chrono_like_now() -> String {
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}
