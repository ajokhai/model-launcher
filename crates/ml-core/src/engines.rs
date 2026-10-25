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
            Kind::Image | Kind::Video => Engine::SdCli,
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
            // weights + KV cache + compute buffers + the capped prompt cache
            m.size_gb + kv + 0.4 + 0.5
        }
        Kind::Image => m.size_gb * 1.3 + 1.2,
        // frames are decoded in tiles, so most of the cost is the weights plus the text encoder
        Kind::Video => m.size_gb * 1.2 + 1.5,
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
    /// What the model was expected to need when it was loaded, for fitting the next one in.
    need_gb: f64,
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
    /// Loaded chat models by id. Several can be loaded at once while they fit the budget and the
    /// "models at once" limit; past that, the least recently used one is unloaded.
    chat: Mutex<Vec<ChatServer>>,
    /// Loads happen one at a time, so two can't both decide the same memory is free.
    loading: Mutex<()>,
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
            chat: Mutex::new(Vec::new()),
            loading: Mutex::new(()),
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

    /// Loaded chat models, most recently used first.
    pub async fn running(&self) -> Vec<Running> {
        let c = self.chat.lock().await;
        let mut v: Vec<Running> = c
            .iter()
            .map(|c| Running {
                id: c.model.id.clone(),
                name: c.model.name.clone(),
                port: c.port,
                pid: c.pid,
                memory_gb: system::process_memory_gb(c.pid).unwrap_or(0.0),
                idle_secs: c.last_used.elapsed().as_secs(),
                ready: c.ready,
                vision: c.model.projector.is_some(),
            })
            .collect();
        v.sort_by_key(|r| r.idle_secs);
        v
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
        let chat: f64 = self.running().await.iter().map(|r| r.memory_gb).sum();
        chat + self.current_job().map(|j| j.memory_gb).unwrap_or(0.0)
    }

    fn write_pids(c: &[ChatServer]) {
        let pids: Vec<String> = c.iter().map(|c| c.pid.to_string()).collect();
        if pids.is_empty() {
            let _ = std::fs::remove_file(pid_file());
        } else {
            let _ = std::fs::write(pid_file(), pids.join("\n"));
        }
    }

    /// Makes sure `m` is loaded and returns its port. Other loaded models stay loaded while there's
    /// room; when there isn't, the least recently used are unloaded first.
    pub async fn ensure_chat(&self, m: &Installed) -> Result<u16> {
        if m.kind != Kind::Chat {
            bail!("{} isn't a chat model", m.name);
        }
        if let Some((port, ready)) = self.touch(&m.id).await {
            if !ready {
                self.wait_ready(port).await?;
            }
            return Ok(port);
        }
        let _one_load = self.loading.lock().await;
        if let Some((port, _)) = self.touch(&m.id).await {
            return Ok(port);
        }

        let s = self.settings.read().await.clone();
        let need = estimate_gb(m, &s);
        if need > s.memory_budget_gb {
            bail!(
                "{} needs about {:.1} GB, more than your {:.0} GB RAM budget. Raise the budget in Settings or pick a smaller model.",
                m.name, need, s.memory_budget_gb
            );
        }
        // make room: unload least recently used models until this one fits the budget and the limit
        loop {
            let mut c = self.chat.lock().await;
            let used: f64 = c.iter().map(|c| c.need_gb).sum();
            let fits = used + need <= s.memory_budget_gb && (c.len() as u32) < s.effective_max_loaded();
            if fits || c.is_empty() {
                break;
            }
            let lru = (0..c.len()).max_by_key(|&i| c[i].last_used.elapsed()).unwrap();
            let old = c.remove(lru);
            Self::write_pids(&c);
            drop(c);
            self.note(format!("Unloaded {} to make room for {}", old.model.name, m.name));
            stop_child(old.child).await;
        }
        let free = system::available_memory_gb() + self.current_job().map(|j| j.memory_gb).unwrap_or(0.0);
        if need > free + 1.0 {
            bail!(
                "{} needs about {:.1} GB but only {:.1} GB is free right now. Close some apps or unload a model, then try again.",
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
            .args(["--jinja", "--parallel", "1"])
            // llama.cpp keeps old prompts in RAM (up to 8 GB by default); cap it so a long session
            // can't grow a model past the budget. 512 MB still covers an agent's recent turns.
            .args(["--cache-ram", "512"]);
        let child = spawn_logged(cmd, &format!("chat-{}", m.id))?;
        let pid = child.id().unwrap_or(0);
        if s.low_priority {
            system::make_low_priority(pid);
        }
        self.note(format!("Loading {} ({:.1} GB expected)", m.name, need));
        {
            let mut c = self.chat.lock().await;
            c.push(ChatServer { model: m.clone(), child, port, pid, need_gb: need, last_used: Instant::now(), ready: false });
            Self::write_pids(&c);
        }
        if let Err(e) = self.wait_ready(port).await {
            self.stop_model(&m.id, "it didn't start").await;
            return Err(e);
        }
        if let Some(c) = self.chat.lock().await.iter_mut().find(|c| c.port == port) {
            c.ready = true;
        }
        self.note(format!("{} is ready", m.name));
        Ok(port)
    }

    /// Marks a loaded model as just used; returns its port and whether it's ready.
    async fn touch(&self, id: &str) -> Option<(u16, bool)> {
        let mut c = self.chat.lock().await;
        let cs = c.iter_mut().find(|c| c.model.id == id)?;
        if cs.child.try_wait().ok().flatten().is_some() {
            return None;
        }
        cs.last_used = Instant::now();
        Some((cs.port, cs.ready))
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
                match c.iter_mut().find(|c| c.port == port) {
                    Some(c) => {
                        if let Ok(Some(status)) = c.child.try_wait() {
                            bail!("the model server stopped while loading ({status}); see {}",
                                logs_dir().join(format!("chat-{}.log", c.model.id)).display());
                        }
                    }
                    None => bail!("the model was unloaded while starting"),
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        bail!("the model took more than two minutes to load")
    }

    pub async fn touch_chat(&self, id: &str) {
        self.touch(id).await;
    }

    pub async fn stop_model(&self, id: &str, why: &str) {
        let removed = {
            let mut c = self.chat.lock().await;
            let i = c.iter().position(|c| c.model.id == id);
            let r = i.map(|i| c.remove(i));
            Self::write_pids(&c);
            r
        };
        if let Some(cs) = removed {
            self.note(format!("Unloaded {}: {why}", cs.model.name));
            stop_child(cs.child).await;
        }
    }

    /// Unloads every chat model.
    pub async fn stop_chat(&self, why: &str) {
        let all: Vec<ChatServer> = {
            let mut c = self.chat.lock().await;
            let v = std::mem::take(&mut *c);
            Self::write_pids(&c);
            v
        };
        for cs in all {
            self.note(format!("Unloaded {}: {why}", cs.model.name));
            stop_child(cs.child).await;
        }
    }

    /// Runs a voice or image job to completion, inside the RAM budget. If the job plus the loaded
    /// chat models would go over, the least recently used chat models are unloaded first.
    pub async fn run_job(&self, m: &Installed, label: &str, mut cmd: Command) -> Result<()> {
        let _permit = self.jobs.acquire().await?;
        let s = self.settings.read().await.clone();
        let need = estimate_gb(m, &s);
        if need > s.memory_budget_gb {
            bail!("{} needs about {:.1} GB, more than your {:.0} GB RAM budget.", m.name, need, s.memory_budget_gb);
        }
        loop {
            let running = self.running().await;
            let chat_gb: f64 = running.iter().map(|r| r.memory_gb).sum();
            match running.last() {
                Some(lru) if chat_gb + need > s.memory_budget_gb => {
                    self.stop_model(&lru.id, &format!("made room for {}", m.name)).await
                }
                _ => break,
            }
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

    /// Every two seconds: keep the total under the RAM budget (stopping the job if it's the one
    /// that grew, else the least recently used chat model), unload idle chat models, and forget
    /// servers that died on their own.
    pub async fn watchdog(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let s = self.settings.read().await.clone();
            let job = self.current_job();
            let job_gb = job.as_ref().map(|j| j.memory_gb).unwrap_or(0.0);
            let running = self.running().await;
            let chat_gb: f64 = running.iter().map(|r| r.memory_gb).sum();
            // a little grace, so a brief spike while loading doesn't kill a model that fits
            let limit = s.memory_budget_gb * 1.1;
            if chat_gb + job_gb > limit {
                let biggest_chat = running.iter().map(|r| r.memory_gb).fold(0.0, f64::max);
                match (job, running.last()) {
                    (Some(j), _) if job_gb > biggest_chat => {
                        unsafe { libc::kill(j.pid as i32, libc::SIGTERM) };
                        self.note(format!("Stopped {}: it used {:.1} GB, over your {:.0} GB RAM budget", j.label, job_gb, s.memory_budget_gb));
                    }
                    (_, Some(lru)) => {
                        let why = format!("models used {:.1} GB, over your {:.0} GB RAM budget", chat_gb + job_gb, s.memory_budget_gb);
                        self.stop_model(&lru.id, &why).await;
                    }
                    _ => {}
                }
                continue;
            }
            if s.idle_unload_minutes > 0 {
                for r in running.iter().filter(|r| r.ready && r.idle_secs >= s.idle_unload_minutes as u64 * 60) {
                    self.stop_model(&r.id, &format!("idle for {} minutes", s.idle_unload_minutes)).await;
                }
            }
            let mut dead = Vec::new();
            {
                let mut c = self.chat.lock().await;
                c.retain_mut(|cs| match cs.child.try_wait() {
                    Ok(Some(st)) => {
                        dead.push(format!("{} stopped on its own ({st})", cs.model.name));
                        false
                    }
                    _ => true,
                });
                if !dead.is_empty() {
                    Self::write_pids(&c);
                }
            }
            for d in dead {
                self.note(d);
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

/// Stops model servers left behind by a run that crashed or was killed, so their memory comes back.
pub fn reap_orphan() -> Vec<u32> {
    let pids: Vec<i32> = std::fs::read_to_string(pid_file()).unwrap_or_default().lines().filter_map(|l| l.trim().parse().ok()).collect();
    let _ = std::fs::remove_file(pid_file());
    let mut stopped = Vec::new();
    for pid in pids {
        let mut buf = [0u8; 4096];
        let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32) };
        let path = String::from_utf8_lossy(&buf[..n.max(0) as usize]).to_string();
        if n > 0 && path.ends_with("/llama-server") {
            unsafe { libc::kill(pid, libc::SIGTERM) };
            stopped.push(pid as u32);
        }
    }
    stopped
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
