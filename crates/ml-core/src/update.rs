//! Updates from GitHub Releases, the way Computer Use does them: check at launch and every 6 hours;
//! on macOS, download the new DMG and install it once nothing is running (no model loaded, no
//! benchmark, job, download or move), then relaunch. Linux gets a notice with the download link.
//! Settings › Update automatically turns the download and install off; checks still show a notice.

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::settings::support_dir;
use crate::Core;

const API: &str = "https://api.github.com/repos/ajokhai/model-launcher/releases/latest";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, Default, Serialize)]
pub struct UpdateState {
    pub current: String,
    /// The newer version on GitHub, if there is one.
    pub latest: Option<String>,
    pub notes: String,
    pub page: String,
    /// The downloaded DMG, ready to install.
    pub ready: Option<PathBuf>,
    pub downloading: bool,
    pub checked_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Updater {
    state: Mutex<UpdateState>,
    asset: Mutex<Option<(String, String)>>, // (name, download url)
}

fn parse(v: &str) -> Vec<u64> {
    v.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

impl Updater {
    pub fn state(&self) -> UpdateState {
        let mut s = self.state.lock().unwrap().clone();
        s.current = VERSION.into();
        s
    }

    /// Asks GitHub for the latest release. Returns the newer version, if any.
    pub async fn check(&self) -> Result<Option<String>> {
        let client = reqwest::Client::builder().user_agent("model-launcher-updater").timeout(Duration::from_secs(30)).build()?;
        let resp = client.get(API).header("Accept", "application/vnd.github+json").send().await?;
        let stamp = crate::bench::now();
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            self.state.lock().unwrap().checked_at = Some(stamp);
            return Ok(None);
        }
        let rel: Value = resp.error_for_status()?.json().await?;
        let tag = rel["tag_name"].as_str().unwrap_or("").trim_start_matches('v').to_string();
        let newer = !tag.is_empty() && parse(&tag) > parse(VERSION);
        let want = if cfg!(target_os = "macos") { ".dmg" } else if cfg!(target_arch = "aarch64") { "aarch64.AppImage" } else { "amd64.AppImage" };
        let asset = rel["assets"].as_array().into_iter().flatten().find(|a| a["name"].as_str().is_some_and(|n| n.ends_with(want)));
        *self.asset.lock().unwrap() = asset.map(|a| (a["name"].as_str().unwrap_or("").into(), a["browser_download_url"].as_str().unwrap_or("").into()));
        let mut s = self.state.lock().unwrap();
        s.checked_at = Some(stamp);
        s.error = None;
        s.latest = newer.then(|| tag.clone());
        s.notes = rel["body"].as_str().unwrap_or("").trim().to_string();
        s.page = rel["html_url"].as_str().unwrap_or("https://github.com/ajokhai/model-launcher/releases").to_string();
        Ok(s.latest.clone())
    }

    /// Downloads the new DMG (macOS) into Application Support › updates.
    async fn download(&self) -> Result<PathBuf> {
        let (name, url) = self.asset.lock().unwrap().clone().ok_or_else(|| anyhow!("the release has no download for this computer"))?;
        let dir = support_dir().join("updates");
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join(&name);
        self.state.lock().unwrap().downloading = true;
        let result = async {
            let bytes = reqwest::Client::builder().user_agent("model-launcher-updater").build()?.get(&url).send().await?.error_for_status()?.bytes().await?;
            std::fs::write(&dest, &bytes)?;
            anyhow::Ok(())
        }
        .await;
        let mut s = self.state.lock().unwrap();
        s.downloading = false;
        result?;
        s.ready = Some(dest.clone());
        Ok(dest)
    }
}

/// The running app bundle (…/Model Launcher.app), when it is one.
fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?.to_path_buf();
    (app.extension().is_some_and(|e| e == "app")).then_some(app)
}

/// Whether installing now would interrupt anything.
async fn busy(core: &Core) -> bool {
    !core.engines.running().await.is_empty()
        || core.engines.current_job().is_some()
        || core.bench.current().is_some_and(|r| r.running)
        || core.mover.busy()
        || core.downloads.all().values().any(|d| !d.finished && d.error.is_none())
}

/// Swaps in the downloaded version and relaunches: a small script waits for this process to quit,
/// copies the app out of the DMG over this one, and opens it.
pub async fn install(core: &Arc<Core>) -> Result<()> {
    let dmg = core.updater.state().ready.ok_or_else(|| anyhow!("no update has been downloaded"))?;
    let app = bundle().ok_or_else(|| anyhow!("this copy isn't an installed app, so it can't update itself"))?;
    if !cfg!(target_os = "macos") {
        bail!("updates install themselves on macOS only");
    }
    let script = support_dir().join("updates").join("install.sh");
    let (app, dmg) = (app.display().to_string(), dmg.display().to_string());
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/zsh
while kill -0 {pid} 2>/dev/null; do sleep 0.3; done
MNT=$(mktemp -d /tmp/ml-update-mnt.XXXX)
hdiutil attach -nobrowse -readonly -noautoopen -noverify -mountpoint "$MNT" "{dmg}" >/dev/null
if [ -d "$MNT/Model Launcher.app" ]; then
  rm -rf "{app}.new"
  ditto "$MNT/Model Launcher.app" "{app}.new" && rm -rf "{app}" && mv "{app}.new" "{app}"
  xattr -dr com.apple.quarantine "{app}" 2>/dev/null
fi
hdiutil detach "$MNT" -quiet || true
rm -f "{dmg}"
open "{app}"
"#,
            pid = std::process::id()
        ),
    )?;
    std::process::Command::new("/bin/zsh")
        .arg(&script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    core.engines.note(format!("Installing Model Launcher {}…", core.updater.state().latest.unwrap_or_default()));
    core.engines.shutdown().await;
    std::process::exit(0);
}

/// Checks at launch and every 6 hours; with Update automatically on, downloads and then installs
/// as soon as nothing is running.
pub async fn run(core: Arc<Core>) {
    tokio::time::sleep(Duration::from_secs(20)).await;
    let mut last_check: Option<std::time::Instant> = None;
    loop {
        if last_check.is_none_or(|t| t.elapsed() > Duration::from_secs(6 * 3600)) {
            last_check = Some(std::time::Instant::now());
            match core.updater.check().await {
                Ok(Some(v)) => core.engines.note(format!("Model Launcher {v} is available")),
                Ok(None) => {}
                Err(e) => core.updater.state.lock().unwrap().error = Some(format!("{e:#}")),
            }
        }
        let auto = core.settings.read().await.auto_update;
        let st = core.updater.state();
        if auto && cfg!(target_os = "macos") && bundle().is_some() && st.latest.is_some() {
            if st.ready.is_none() && !st.downloading {
                if let Err(e) = core.updater.download().await {
                    core.updater.state.lock().unwrap().error = Some(format!("couldn't download the update: {e:#}"));
                }
            } else if st.ready.is_some() && !busy(&core).await {
                let _ = install(&core).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// "Check for updates" in Settings: checks now and, with Update automatically on, starts the download.
pub async fn check_now(core: &Arc<Core>) -> Result<UpdateState> {
    core.updater.check().await?;
    let st = core.updater.state();
    if st.latest.is_some() && st.ready.is_none() && cfg!(target_os = "macos") && bundle().is_some() {
        let c = core.clone();
        tokio::spawn(async move {
            if let Err(e) = c.updater.download().await {
                c.updater.state.lock().unwrap().error = Some(format!("couldn't download the update: {e:#}"));
            }
        });
    }
    Ok(core.updater.state())
}
