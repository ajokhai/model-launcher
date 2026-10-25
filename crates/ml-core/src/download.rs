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

    async fn fetch(&self, client: &reqwest::Client, id: &str, url: &str, dest: &Path, before_gb: f64) -> Result<u64> {
        let part = dest.with_extension("part");
        let resp = client.get(url).send().await?.error_for_status()?;
        if let Some(len) = resp.content_length() {
            let need = before_gb + len as f64 / GB;
            self.set(id, |p| if p.total_gb < need { p.total_gb = need });
        }
        let mut file = tokio::fs::File::create(&part).await?;
        let mut stream = resp.bytes_stream();
        let mut got: u64 = 0;
        while let Some(chunk) = stream.next().await {
            if self.cancelled.lock().unwrap().iter().any(|c| c == id) {
                drop(file);
                let _ = tokio::fs::remove_file(&part).await;
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
        drop(file);
        tokio::fs::rename(&part, dest).await?;
        Ok(got)
    }
}

/// Installs an engine. llama.cpp and whisper.cpp come from Homebrew when it's there; the image
/// engine (stable-diffusion.cpp) comes from its official macOS release into Model Launcher's own
/// engines folder.
pub async fn install_engine(engine: Engine) -> Result<String> {
    match engine {
        Engine::LlamaServer | Engine::LlamaTts => brew("llama.cpp").await,
        Engine::WhisperCli => brew("whisper-cpp").await,
        Engine::SdCli => install_sd().await,
    }
}

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

async fn install_sd() -> Result<String> {
    let client = reqwest::Client::builder().user_agent("model-launcher").build()?;
    let rel: serde_json::Value = client
        .get("https://api.github.com/repos/leejet/stable-diffusion.cpp/releases/latest")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let asset = rel["assets"]
        .as_array()
        .and_then(|a| {
            a.iter().find(|x| {
                let n = x["name"].as_str().unwrap_or("");
                n.contains("Darwin") && n.contains("arm64") && n.ends_with(".zip")
            })
        })
        .ok_or_else(|| anyhow!("no macOS build in the latest stable-diffusion.cpp release"))?;
    let url = asset["browser_download_url"].as_str().unwrap_or_default();
    let bytes = client.get(url).send().await?.error_for_status()?.bytes().await?;
    let dir = engines_dir();
    std::fs::create_dir_all(&dir)?;
    let zip = dir.join("sd.zip");
    std::fs::write(&zip, &bytes)?;
    let st = tokio::process::Command::new("/usr/bin/ditto").arg("-x").arg("-k").arg(&zip).arg(&dir).status().await?;
    let _ = std::fs::remove_file(&zip);
    if !st.success() {
        bail!("couldn't unpack the image engine");
    }
    // the release keeps its files in the zip's top level or a folder; bring sd-cli and its library up
    for entry in walk(&dir) {
        let name = entry.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if (name == "sd-cli" || name.ends_with(".dylib")) && entry.parent() != Some(dir.as_path()) {
            let _ = std::fs::rename(&entry, dir.join(&name));
        }
    }
    let _ = tokio::process::Command::new("/usr/bin/xattr").arg("-dr").arg("com.apple.quarantine").arg(&dir).status().await;
    Engine::SdCli.path().context("the image engine didn't unpack where expected")?;
    Ok(format!("Installed stable-diffusion.cpp {}", rel["tag_name"].as_str().unwrap_or("")))
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
