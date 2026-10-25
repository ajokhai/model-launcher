//! Where models live: this Mac's drive or an external SSD. Moving the Models folder to a drive keeps
//! ~/Models as a link to it, so it's still easy to find.
//!
//! macOS doesn't let apps put swap on an external drive. What works instead is the way llama.cpp
//! loads weights: it maps the model file into memory, so with "Use disk as extra memory" a model
//! bigger than the RAM budget still loads, and macOS reads its weights from the drive as needed
//! instead of keeping them all in RAM. A fast SSD (Thunderbolt or USB4) makes that bearable.

use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::system;

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Clone, Debug, Default, Serialize)]
pub struct MoveProgress {
    pub to: PathBuf,
    pub done_gb: f64,
    pub total_gb: f64,
    pub finished: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Mover {
    state: Mutex<Option<MoveProgress>>,
}

impl Mover {
    pub fn progress(&self) -> Option<MoveProgress> {
        self.state.lock().unwrap().clone()
    }

    pub fn busy(&self) -> bool {
        self.progress().is_some_and(|p| !p.finished && p.error.is_none())
    }

    fn set(&self, f: impl FnOnce(&mut MoveProgress)) {
        if let Some(p) = self.state.lock().unwrap().as_mut() {
            f(p);
        }
    }

    /// Moves every model from `from` to `to` in the background, then calls `done` with the new
    /// folder. Files are copied and checked before the originals are deleted.
    pub fn start(self: &Arc<Self>, from: PathBuf, to: PathBuf, done: impl FnOnce(PathBuf) + Send + 'static) -> Result<()> {
        if self.busy() {
            bail!("models are already being moved");
        }
        let files = list_files(&from);
        let total: u64 = files.iter().map(|(_, n)| n).sum();
        let free = system::free_disk_gb(&to);
        if total as f64 / GB + 0.5 > free {
            bail!("the models take {:.1} GB and that drive has {:.1} GB free", total as f64 / GB, free);
        }
        *self.state.lock().unwrap() = Some(MoveProgress { to: to.clone(), total_gb: total as f64 / GB, ..Default::default() });
        let me = self.clone();
        tokio::task::spawn_blocking(move || match me.copy_all(&from, &to, &files) {
            Ok(()) => {
                for (f, _) in &files {
                    let _ = std::fs::remove_file(f);
                }
                remove_empty_dirs(&from);
                done(to);
                me.set(|p| p.finished = true);
            }
            Err(e) => me.set(|p| p.error = Some(format!("{e:#}"))),
        });
        Ok(())
    }

    fn copy_all(&self, from: &Path, to: &Path, files: &[(PathBuf, u64)]) -> Result<()> {
        let mut done: u64 = 0;
        let mut buf = vec![0u8; 8 << 20];
        for (src, len) in files {
            let dest = to.join(src.strip_prefix(from)?);
            if let Some(dir) = dest.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut r = std::fs::File::open(src)?;
            let mut w = std::fs::File::create(&dest)?;
            loop {
                let n = r.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                w.write_all(&buf[..n])?;
                done += n as u64;
                self.set(|p| p.done_gb = done as f64 / GB);
            }
            w.sync_all()?;
            if std::fs::metadata(&dest)?.len() != *len {
                bail!("{} didn't copy completely", src.display());
            }
        }
        Ok(())
    }
}

fn list_files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        match e.metadata() {
            Ok(m) if m.is_dir() => out.extend(list_files(&p)),
            Ok(m) if m.is_file() && e.file_name() != ".DS_Store" => out.push((p, m.len())),
            _ => {}
        }
    }
    out
}

fn remove_empty_dirs(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                remove_empty_dirs(&e.path());
            }
        }
    }
    let _ = std::fs::remove_file(dir.join(".DS_Store"));
    let _ = std::fs::remove_dir(dir);
}

/// ~/Models: the easy-to-find folder, or a link to the models on an external drive.
pub fn home_models() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Models")
}

/// Points ~/Models at `target` (a link), or makes it a real folder again when `target` is ~/Models.
pub fn relink_home(target: &Path) {
    let home = home_models();
    let is_link = std::fs::symlink_metadata(&home).is_ok_and(|m| m.file_type().is_symlink());
    if target == home {
        if is_link {
            let _ = std::fs::remove_file(&home);
        }
        let _ = std::fs::create_dir_all(&home);
        return;
    }
    if is_link {
        let _ = std::fs::remove_file(&home);
    } else if home.exists() {
        // only an emptied folder is replaced; anything left in it stays put
        remove_empty_dirs(&home);
        if home.exists() {
            return;
        }
    }
    let _ = std::os::unix::fs::symlink(target, &home);
}

/// This Mac's drive plus every connected drive, with free space.
pub fn drives(models_dir: &Path) -> Vec<Value> {
    let mut out = vec![json!({
        "name": "This Mac",
        "path": home_models(),
        "free_gb": system::free_disk_gb(Path::new("/")),
        "external": false,
        "current": !is_on_volume(models_dir),
    })];
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        for e in rd.flatten() {
            let p = e.path();
            // the startup disk shows up here as a link to /
            if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) || !writable(&p) {
                continue;
            }
            let models = p.join("Models");
            out.push(json!({
                "name": e.file_name().to_string_lossy(),
                "path": models,
                "free_gb": system::free_disk_gb(&p),
                "external": true,
                "current": models_dir.starts_with(&p),
            }));
        }
    }
    out
}

fn is_on_volume(p: &Path) -> bool {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).starts_with("/Volumes")
}

fn writable(p: &Path) -> bool {
    let Ok(c) = std::ffi::CString::new(p.to_string_lossy().as_bytes()) else { return false };
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn moves_and_checks() {
        let base = std::env::temp_dir().join(format!("ml-move-{}", std::process::id()));
        let (from, to) = (base.join("a/Models"), base.join("b/Models"));
        std::fs::create_dir_all(from.join("Chat/m")).unwrap();
        std::fs::write(from.join("Chat/m/w.gguf"), vec![7u8; 20 << 20]).unwrap();
        std::fs::write(from.join("About this folder.txt"), "hi").unwrap();
        let mover = Arc::new(Mover::default());
        let (tx, rx) = tokio::sync::oneshot::channel();
        mover.start(from.clone(), to.clone(), move |d| { let _ = tx.send(d); }).unwrap();
        assert_eq!(rx.await.unwrap(), to);
        assert_eq!(std::fs::metadata(to.join("Chat/m/w.gguf")).unwrap().len(), 20 << 20);
        assert!(!from.exists());
        println!("{:#?}", drives(&home_models()));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
