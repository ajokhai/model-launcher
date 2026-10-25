//! Downloads models into their folder (with a disk-space check first) and installs engines.

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;

use crate::catalog;
use crate::engines::{engines_dir, Engine};
use crate::settings::Settings;
use crate::system;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Progress {
    pub done_gb: f64,
    pub total_gb: f64,
    pub error: Option<String>,
    pub finished: bool,
}

#[derive(Default)]
pub struct Downloads {
    state: Mutex<HashMap<String, Progress>>,
    cancelled: Mutex<Vec<String>>,
}

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

impl Downloads {
    pub fn all(&self) -> HashMap<String, Progress> {
        self.state.lock().unwrap().clone()
    }

    pub fn cancel(&self, id: &str) {
        self.cancelled.lock().unwrap().push(id.to_string());
    }

    fn set(&self, id: &str, f: impl FnOnce(&mut Progress)) {
        f(self.state.lock().unwrap().entry(id.to_string()).or_default());
    }

    /// Starts downloading a catalog model in the background. Fails at once if there isn't room.
    pub fn start(self: &Arc<Self>, id: &str, settings: Arc<RwLock<Settings>>) -> Result<()> {
        let model = catalog::get(id).ok_or_else(|| anyhow!("unknown model {id}"))?;
        if self.state.lock().unwrap().get(id).is_some_and(|p| !p.finished && p.error.is_none()) {
            return Ok(());
        }
        let s = settings.try_read().map(|s| s.clone()).unwrap_or_default();
        s.models_drive_ready()?;
        let dir = s.kind_dir(model.kind).join(model.id);
        std::fs::create_dir_all(&dir)?;
        let free = system::free_disk_gb(&dir);
        let need = model.size_gb() + 1.0;
        if need > free {
            bail!("{} needs {:.1} GB of disk and only {:.1} GB is free.", model.name, model.size_gb(), free);
        }
        self.cancelled.lock().unwrap().retain(|c| c != id);
        self.set(id, |p| *p = Progress { total_gb: model.size_gb(), ..Default::default() });
        let me = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut done_before = 0.0;
            for file in &model.files {
                let dest = dir.join(file.local_name());
                if dest.is_file() {
                    done_before += file.size_gb;
                    continue;
                }
                match me.fetch(&client, &id, &file.url(), &dest, done_before).await {
                    Ok(bytes) => done_before += bytes as f64 / GB,
                    Err(e) => {
                        me.set(&id, |p| p.error = Some(e.to_string()));
                        return;
                    }
                }
            }
            me.set(&id, |p| {
                p.finished = true;
                p.done_gb = p.total_gb;
            });
        });
        Ok(())
    }

    /// Downloads model files from any https link (a Hugging Face "resolve" or "blob" link, or any
    /// direct link to a .gguf, .bin or .safetensors) into `dir`, tracked under `id`.
    pub fn start_urls(self: &Arc<Self>, id: &str, dir: std::path::PathBuf, urls: Vec<String>) -> Result<()> {
        let mut files = Vec::new();
        for u in urls.iter().map(|u| u.trim()).filter(|u| !u.is_empty()) {
            let url = normalize_url(u)?;
            let name = url.rsplit('/').next().unwrap_or_default().split('?').next().unwrap_or_default().to_string();
            if !(name.ends_with(".gguf") || name.ends_with(".bin") || name.ends_with(".safetensors")) {
                bail!("{name} isn't a model file. Link straight to a .gguf, .bin or .safetensors file.");
            }
            files.push((url, name));
        }
        if files.is_empty() {
            bail!("paste at least one link to a model file");
        }
        std::fs::create_dir_all(&dir)?;
        if system::free_disk_gb(&dir) < 2.0 {
            bail!("less than 2 GB of disk is free; make room first");
        }
        self.cancelled.lock().unwrap().retain(|c| c != id);
        self.set(id, |p| *p = Progress::default());
        let me = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            // sizes come from the server, so the progress bar is real before the first byte
            let mut total = 0.0;
            for (url, _) in &files {
                if let Ok(r) = client.head(url).send().await {
                    // Hugging Face redirects to a CDN and states the size in x-linked-size
                    let linked = r.headers().get("x-linked-size").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
                    total += linked.or(r.content_length()).unwrap_or(0) as f64 / GB;
                }
            }
            if total > 0.0 && total + 1.0 > system::free_disk_gb(&dir) {
                me.set(&id, |p| p.error = Some(format!("needs {total:.1} GB of disk, more than is free")));
                return;
            }
            me.set(&id, |p| p.total_gb = total.max(0.01));
            let mut done = 0.0;
            for (url, name) in &files {
                match me.fetch(&client, &id, url, &dir.join(name), done).await {
                    Ok(bytes) => done += bytes as f64 / GB,
                    Err(e) => {
                        me.set(&id, |p| p.error = Some(e.to_string()));
                        return;
                    }
                }
            }
            me.set(&id, |p| {
                p.finished = true;
                p.done_gb = done;
                p.total_gb = done;
            });
        });
        Ok(())
    }

    /// Downloads one file to `dest`, resuming a partial `.part` from where it stopped and retrying
    /// dropped connections (up to 8 times, with a growing pause), so a big model survives flaky Wi-Fi.
    async fn fetch(&self, client: &reqwest::Client, id: &str, url: &str, dest: &Path, before_gb: f64) -> Result<u64> {
        let part = dest.with_extension("part");
        let mut tries = 0;
        loop {
            match self.fetch_once(client, id, url, &part, before_gb).await {
                Ok(()) => break,
                Err(e) if e.to_string() == "Cancelled" => {
                    let _ = tokio::fs::remove_file(&part).await;
                    return Err(e);
                }
                Err(e) if tries < 8 => {
                    tries += 1;
                    self.set(id, |p| p.error = None);
                    eprintln!("download {id}: {e:#}; retry {tries} of 8");
                    tokio::time::sleep(std::time::Duration::from_secs(2 * tries)).await;
                }
                Err(e) => return Err(e.context("the download kept dropping; press Download to resume")),
            }
        }
        let got = tokio::fs::metadata(&part).await?.len();
        tokio::fs::rename(&part, dest).await?;
        Ok(got)
    }

    async fn fetch_once(&self, client: &reqwest::Client, id: &str, url: &str, part: &Path, before_gb: f64) -> Result<()> {
        let have = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
        let mut req = client.get(url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let resp = req.send().await?;
        if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            return Ok(()); // already complete
        }
        let resp = resp.error_for_status()?;
        let resumed = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        let mut got: u64 = if resumed { have } else { 0 };
        if let Some(len) = resp.content_length() {
            let need = before_gb + (got + len) as f64 / GB;
            self.set(id, |p| if p.total_gb < need { p.total_gb = need });
        }
        let mut file = if resumed {
            tokio::fs::OpenOptions::new().append(true).open(part).await?
        } else {
            tokio::fs::File::create(part).await?
        };
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            if self.cancelled.lock().unwrap().iter().any(|c| c == id) {
                bail!("Cancelled");
            }
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            got += chunk.len() as u64;
            if got % (8 << 20) < chunk.len() as u64 {
                self.set(id, |p| p.done_gb = before_gb + got as f64 / GB);
            }
        }
        file.flush().await?;
        Ok(())
    }
}

/// Installs an engine. On macOS llama.cpp and whisper.cpp come from Homebrew and the image engine
/// (stable-diffusion.cpp) from its official release; on Linux all three come from their official
/// releases. Release builds go into Model Launcher's own engines folder.
#[cfg(target_os = "macos")]
pub async fn install_engine(engine: Engine) -> Result<String> {
    match engine {
        Engine::LlamaServer | Engine::LlamaTts => brew("llama.cpp").await,
        Engine::WhisperCli => brew("whisper-cpp").await,
        Engine::SdCli => install_release("leejet/stable-diffusion.cpp", Engine::SdCli, &|n| n.contains("Darwin") && n.contains("arm64") && n.ends_with(".zip")).await,
    }
}

#[cfg(target_os = "linux")]
pub async fn install_engine(engine: Engine) -> Result<String> {
    let arm = cfg!(target_arch = "aarch64");
    // Vulkan runs on NVIDIA, AMD and Intel GPUs alike; without its loader, use the CPU build
    let vulkan = ["/usr/lib/x86_64-linux-gnu/libvulkan.so.1", "/usr/lib/aarch64-linux-gnu/libvulkan.so.1", "/usr/lib64/libvulkan.so.1", "/usr/lib/libvulkan.so.1"]
        .iter()
        .any(|p| Path::new(p).exists());
    let arch = if arm { "arm64" } else { "x64" };
    match engine {
        Engine::LlamaServer | Engine::LlamaTts => {
            let want = if vulkan { format!("bin-ubuntu-vulkan-{arch}.tar.gz") } else { format!("bin-ubuntu-{arch}.tar.gz") };
            install_release("ggml-org/llama.cpp", engine, &|n| n.starts_with("llama-") && n.ends_with(&want)).await
        }
        Engine::WhisperCli => {
            let want = format!("whisper-bin-ubuntu-{arch}.tar.gz");
            install_release("ggml-org/whisper.cpp", engine, &|n| n == want).await
        }
        Engine::SdCli => {
            let cpu = if arm { "aarch64" } else { "x86_64" };
            install_release("leejet/stable-diffusion.cpp", engine, &|n| {
                n.contains("Linux") && n.contains(cpu) && n.ends_with(".zip") && !n.contains("rocm") && (n.contains("vulkan") == vulkan)
            })
            .await
        }
    }
}

#[cfg(target_os = "macos")]
async fn brew(formula: &str) -> Result<String> {
    let brew = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .ok_or_else(|| anyhow!("Homebrew isn't installed. Get it from brew.sh, then try again."))?;
    let out = tokio::process::Command::new(brew).args(["install", formula]).output().await?;
    if !out.status.success() {
        bail!("brew install {formula} failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""));
    }
    Ok(format!("Installed {formula}"))
}

/// Downloads a project's latest release build (the first asset `pick` accepts) into the engines
/// folder, with every binary and library at its top level.
async fn install_release(repo: &str, engine: Engine, pick: &(dyn Fn(&str) -> bool + Sync)) -> Result<String> {
    let client = reqwest::Client::builder().user_agent("model-launcher").build()?;
    // llama.cpp tags a release for every build; take the newest that carries this platform's files
    let rels: Vec<serde_json::Value> = client
        .get(format!("https://api.github.com/repos/{repo}/releases?per_page=10"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let (rel, asset) = rels
        .iter()
        .find_map(|r| r["assets"].as_array()?.iter().find(|x| pick(x["name"].as_str().unwrap_or(""))).map(|a| (r, a)))
        .ok_or_else(|| anyhow!("no build for this computer in the latest {repo} releases"))?;
    let name = asset["name"].as_str().unwrap_or_default();
    let url = asset["browser_download_url"].as_str().unwrap_or_default();
    let bytes = client.get(url).send().await?.error_for_status()?.bytes().await?;
    let dir = engines_dir();
    let tmp = dir.join(".unpack");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let archive = dir.join(name);
    std::fs::write(&archive, &bytes)?;
    let st = if name.ends_with(".zip") {
        tokio::process::Command::new("unzip").arg("-q").arg("-o").arg(&archive).arg("-d").arg(&tmp).status().await?
    } else {
        tokio::process::Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&tmp).status().await?
    };
    let _ = std::fs::remove_file(&archive);
    if !st.success() {
        bail!("couldn't unpack {name}");
    }
    // releases keep files at the top level or in a folder; bring binaries and libraries up
    for entry in walk(&tmp) {
        let file = entry.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let _ = std::fs::rename(&entry, dir.join(&file));
    }
    let _ = std::fs::remove_dir_all(&tmp);
    #[cfg(target_os = "macos")]
    let _ = tokio::process::Command::new("/usr/bin/xattr").arg("-dr").arg("com.apple.quarantine").arg(&dir).status().await;
    #[cfg(unix)]
    if let Some(p) = Engine::path(engine) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755));
    }
    Engine::path(engine).context("the engine didn't unpack where expected")?;
    Ok(format!("Installed {} {}", repo.split('/').nth(1).unwrap_or(repo), rel["tag_name"].as_str().unwrap_or("")))
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// Turns a Hugging Face page link (…/blob/main/file.gguf) into its download link (…/resolve/…).
fn normalize_url(u: &str) -> Result<String> {
    if !u.starts_with("https://") {
        bail!("{u} isn't an https link");
    }
    Ok(if u.contains("huggingface.co/") { u.replacen("/blob/", "/resolve/", 1) } else { u.to_string() })
}

/// A folder name that's safe and readable: letters, digits, spaces, dots, dashes and underscores.
pub fn folder_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| !"()[]{}".contains(*c))
        .map(|c| if c.is_alphanumeric() || " ._-".contains(c) { c } else { '-' })
        .collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean = clean.trim_matches(|c: char| c == '.' || c == ' ' || c == '-').to_string();
    if clean.is_empty() { "My model".into() } else { clean }
}
