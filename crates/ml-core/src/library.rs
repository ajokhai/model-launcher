//! The models on this Mac. Everything lives under one folder (~/Models by default):
//!
//!   ~/Models/Chat/<model>/   ~/Models/Voice/<model>/   ~/Models/Image/<model>/
//!
//! One subfolder per model, so a model is added by dropping its folder in and removed by deleting it.

use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::catalog::{self, Kind, Role};
use crate::settings::Settings;

#[derive(Clone, Debug, Serialize)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub maker: String,
    pub dir: PathBuf,
    pub weights: PathBuf,
    pub projector: Option<PathBuf>,
    pub vocoder: Option<PathBuf>,
    pub size_gb: f64,
    pub vision: bool,
    pub from_catalog: bool,
}

const KINDS: [Kind; 3] = [Kind::Chat, Kind::Transcribe, Kind::Image];

fn size_gb(p: &Path) -> f64 {
    std::fs::metadata(p).map(|m| m.len() as f64 / 1024f64.powi(3)).unwrap_or(0.0)
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    v.sort();
    v
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn is_model_file(p: &Path) -> bool {
    let n = name_of(p).to_lowercase();
    n.ends_with(".gguf") || n.ends_with(".safetensors") || (n.ends_with(".bin") && n.starts_with("ggml-"))
}

/// Reads one model folder. Catalog folders are recognised by their files; anything else is read by
/// file name, so models from elsewhere work too.
fn read_model(folder_kind: Kind, dir: &Path) -> Option<Installed> {
    let id = name_of(dir);
    let files: Vec<PathBuf> = files_in(dir).into_iter().filter(|p| is_model_file(p)).collect();
    if files.is_empty() {
        return None;
    }
    let find = |n: &str| files.iter().find(|p| name_of(p) == n).cloned();

    if let Some(m) = catalog::get(&id) {
        if m.files.iter().all(|f| find(f.name).is_some()) {
            let pick = |role: Role| m.files.iter().find(|f| f.role == role).and_then(|f| find(f.name));
            let weights = pick(Role::Weights)?;
            let projector = pick(Role::Projector);
            let vocoder = pick(Role::Vocoder);
            return Some(Installed {
                id,
                name: m.name.to_string(),
                kind: m.kind,
                maker: m.maker.to_string(),
                dir: dir.to_path_buf(),
                size_gb: files.iter().map(|p| size_gb(p)).sum(),
                weights,
                projector,
                vocoder,
                vision: m.vision,
                from_catalog: true,
            });
        }
    }

    let lower = |p: &PathBuf| name_of(p).to_lowercase();
    let projector = files.iter().find(|p| lower(p).starts_with("mmproj")).cloned();
    let vocoder = files.iter().find(|p| lower(p).contains("wavtokenizer")).cloned();
    let weights = files
        .iter()
        .find(|p| Some(*p) != projector.as_ref() && Some(*p) != vocoder.as_ref())?
        .clone();
    let kind = match folder_kind {
        Kind::Chat => Kind::Chat,
        Kind::Image => Kind::Image,
        _ if lower(&weights).ends_with(".bin") => Kind::Transcribe,
        _ => Kind::Speak,
    };
    Some(Installed {
        name: id.clone(),
        id,
        kind,
        maker: String::new(),
        dir: dir.to_path_buf(),
        size_gb: files.iter().map(|p| size_gb(p)).sum(),
        vision: projector.is_some(),
        weights,
        projector,
        vocoder,
        from_catalog: false,
    })
}

pub fn scan(s: &Settings) -> Vec<Installed> {
    let mut out = Vec::new();
    for kind in KINDS {
        let root = s.kind_dir(kind);
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        out.extend(dirs.iter().filter_map(|d| read_model(kind, d)));
    }
    out
}

pub fn find(s: &Settings, id: &str) -> Option<Installed> {
    scan(s).into_iter().find(|m| m.id == id)
}

/// Creates the folder layout and moves stray model files (left in ~/Models itself or directly in a
/// kind folder) into a folder of their own. Files that belong to a catalog model go to its folder.
pub fn tidy(s: &Settings) -> Vec<String> {
    let mut moved = Vec::new();
    for kind in KINDS {
        let _ = std::fs::create_dir_all(s.kind_dir(kind));
    }
    let _ = std::fs::write(
        s.models_dir.join("About this folder.txt"),
        "Model Launcher keeps every model here.\n\n\
         Chat/   language models (llama.cpp .gguf)\n\
         Voice/  speech to text (whisper .bin) and text to speech (.gguf)\n\
         Image/  image models (.gguf or .safetensors)\n\n\
         Each model has its own folder. Drop a folder in to add a model, delete it to remove one.\n",
    );
    let catalog = catalog::all();
    let mut strays: Vec<PathBuf> = files_in(&s.models_dir);
    for kind in KINDS {
        strays.extend(files_in(&s.kind_dir(kind)));
    }
    for p in strays.into_iter().filter(|p| is_model_file(p)) {
        let n = name_of(&p);
        let (kind, folder) = match catalog.iter().find(|m| m.files.iter().any(|f| f.name == n)) {
            Some(m) => (m.kind, m.id.to_string()),
            None => {
                let l = n.to_lowercase();
                let kind = if l.ends_with(".bin") || l.contains("tts") || l.contains("wavtokenizer") {
                    Kind::Transcribe
                } else if l.contains("diffusion") || l.contains("sd_") || l.ends_with(".safetensors") {
                    Kind::Image
                } else {
                    Kind::Chat
                };
                let stem = n.rsplit_once('.').map(|(a, _)| a).unwrap_or(&n).to_string();
                (kind, stem)
            }
        };
        let dest_dir = s.kind_dir(kind).join(&folder);
        if std::fs::create_dir_all(&dest_dir).is_ok() && std::fs::rename(&p, dest_dir.join(&n)).is_ok() {
            moved.push(format!("{n} → {}/{folder}", kind.folder()));
        }
    }
    moved
}
