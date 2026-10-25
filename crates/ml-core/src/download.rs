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
                let dest = dir.join(file.name);
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

    async fn fetch(&self, client: &reqwest::Client, id: &str, url: &str, dest: &Path, before_gb: f64) -> Result<u64> {
        let part = dest.with_extension("part");
        let resp = client.get(url).send().await?.error_for_status()?;
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
