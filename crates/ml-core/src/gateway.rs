//! The local API on 127.0.0.1 (port 11500 by default).
//!
//! `/v1/*` is OpenAI-compatible, so any app or SDK that speaks OpenAI can use local models: chat
//! (streaming included), audio transcriptions, speech and image generations. Naming a model swaps it
//! in. `/api/*` is what the app's own window and the MCP server use.
//!
//! Only this Mac can reach it, and browser pages can't: requests from any web origin other than the
//! app's own window are refused.

use anyhow::anyhow;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Multipart, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::process::Command;

use crate::catalog::{self, Kind};
use crate::download;
use crate::engines::{estimate_gb, Engine, NotEnoughMemory};
use crate::library::{self, Installed};
use crate::settings::{outputs_dir, Settings};
use crate::storage;
use crate::system;
use crate::web;
use crate::Core;

pub struct ApiError(StatusCode, String);

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        let e = e.into();
        // 507 tells the app it can offer "Run anyway"
        let code = if e.is::<NotEnoughMemory>() { StatusCode::INSUFFICIENT_STORAGE } else { StatusCode::BAD_REQUEST };
        ApiError(code, format!("{e:#}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let kind = if self.0 == StatusCode::INSUFFICIENT_STORAGE { "insufficient_memory" } else { "invalid_request_error" };
        (self.0, Json(json!({ "error": { "message": self.1, "type": kind } }))).into_response()
    }
}

type R<T> = Result<T, ApiError>;

const APP_ORIGINS: [&str; 2] = ["tauri://localhost", "http://tauri.localhost"];

async fn guard(req: Request, next: Next) -> Response {
    let origin = req.headers().get(header::ORIGIN).and_then(|o| o.to_str().ok()).map(str::to_string);
    if let Some(o) = &origin {
        if !APP_ORIGINS.contains(&o.as_str()) {
            return ApiError(StatusCode::FORBIDDEN, "web pages can't use Model Launcher's API".into()).into_response();
        }
    }
    let preflight = req.method() == Method::OPTIONS;
    let mut resp = if preflight { StatusCode::NO_CONTENT.into_response() } else { next.run(req).await };
    if let Some(o) = origin {
        let h = resp.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_str(&o).unwrap());
        h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
        h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("content-type, authorization"));
    }
    resp
}

pub fn router(core: Arc<Core>) -> Router {
    Router::new()
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat))
        .route("/v1/audio/transcriptions", post(transcribe))
        .route("/v1/audio/speech", post(speak))
        .route("/v1/images/generations", post(image))
        .route("/v1/videos/generations", post(video))
        .route("/api/status", get(status))
        .route("/api/library", get(library_list))
        .route("/api/catalog", get(catalog_list))
        .route("/api/launch", post(launch))
        .route("/api/stop", post(stop))
        .route("/api/download", post(download_start))
        .route("/api/download/cancel", post(download_cancel))
        .route("/api/import/url", post(import_url))
        .route("/api/import/file", post(import_file))
        .route("/api/delete", post(delete))
        .route("/api/settings", post(update_settings))
        .route("/api/engines/install", post(install_engine))
        .route("/api/reveal", post(reveal))
        .route("/api/drives", get(drives))
        .route("/api/models/move", post(move_models))
        .route("/api/web/search", post(web_search))
        .route("/api/web/fetch", post(web_fetch))
        .layer(DefaultBodyLimit::max(200 << 20))
        .layer(middleware::from_fn(guard))
        .with_state(core)
}

async fn settings(core: &Core) -> Settings {
    core.settings.read().await.clone()
}

/// Finds an installed model of `kind` by id or name. No name means the loaded chat model, or the
/// first installed one.
async fn resolve(core: &Core, name: Option<&str>, kind: Kind) -> R<Installed> {
    let s = settings(core).await;
    let all: Vec<Installed> = library::scan(&s).into_iter().filter(|m| m.kind == kind).collect();
    let name = name.map(str::trim).filter(|n| !n.is_empty() && *n != "default" && *n != "local");
    let found = match name {
        Some(n) => {
            let l = n.to_lowercase();
            all.iter()
                .find(|m| m.id.to_lowercase() == l || m.name.to_lowercase() == l)
                .or_else(|| all.iter().find(|m| m.id.to_lowercase().starts_with(&l)))
                .cloned()
        }
        None => {
            let running = core.engines.running().await.first().map(|r| r.id.clone());
            all.iter().find(|m| Some(&m.id) == running.as_ref()).or(all.first()).cloned()
        }
    };
    found.ok_or_else(|| {
        let what = match kind {
            Kind::Chat => "chat",
            Kind::Transcribe => "speech-to-text",
            Kind::Speak => "text-to-speech",
            Kind::Image => "image",
            Kind::Video => "video",
        };
        match name {
            Some(n) => ApiError(StatusCode::NOT_FOUND, format!("no {what} model called {n} is installed")),
            None => ApiError(StatusCode::NOT_FOUND, format!("no {what} model is installed; get one in Discover")),
        }
    })
}

async fn models(State(core): State<Arc<Core>>) -> Json<Value> {
    let s = settings(&core).await;
    let data: Vec<Value> = library::scan(&s)
        .iter()
        .map(|m| json!({ "id": m.id, "object": "model", "owned_by": "local", "name": m.name, "kind": m.kind }))
        .collect();
    Json(json!({ "object": "list", "data": data }))
}

async fn chat(State(core): State<Arc<Core>>, body: Bytes) -> R<Response> {
    let v: Value = serde_json::from_slice(&body)?;
    let m = resolve(&core, v["model"].as_str(), Kind::Chat).await?;
    let port = core.engines.ensure_chat(&m).await?;
    core.engines.touch_chat(&m.id).await;
    let resp = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let ctype = resp.headers().get(header::CONTENT_TYPE).cloned();
    let mut out = Response::new(Body::from_stream(resp.bytes_stream()));
    *out.status_mut() = status;
    if let Some(c) = ctype {
        out.headers_mut().insert(header::CONTENT_TYPE, c);
    }
    Ok(out)
}

fn out_path(ext: &str) -> PathBuf {
    let _ = std::fs::create_dir_all(outputs_dir());
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    outputs_dir().join(format!("{}-{:03}.{ext}", t.as_secs(), t.subsec_millis()))
}

fn engine_bin(e: Engine) -> anyhow::Result<PathBuf> {
    e.path().ok_or_else(|| anyhow!("the {} engine isn't installed yet (Settings › Engines)", e.binary()))
}

async fn transcribe(State(core): State<Arc<Core>>, mut form: Multipart) -> R<Json<Value>> {
    let (mut audio, mut model, mut fname) = (None, None, String::from("audio"));
    while let Some(field) = form.next_field().await? {
        match field.name() {
            Some("file") => {
                fname = field.file_name().unwrap_or("audio").to_string();
                audio = Some(field.bytes().await?);
            }
            Some("model") => model = Some(field.text().await?),
            _ => {}
        }
    }
    let audio = audio.ok_or_else(|| anyhow!("send the audio as a multipart field named file"))?;
    let m = resolve(&core, model.as_deref(), Kind::Transcribe).await?;
    let s = settings(&core).await;
    let ext = fname.rsplit_once('.').map(|(_, e)| e).unwrap_or("bin");
    let input = out_path(ext);
    std::fs::write(&input, &audio)?;
    // whisper wants 16 kHz mono WAV; macOS's own converter handles mp3, m4a, wav, aiff and more
    let wav = input.with_extension("16k.wav");
    let st = Command::new("/usr/bin/afconvert").arg("-f").arg("WAVE").arg("-d").arg("LEI16@16000").arg("-c").arg("1")
        .arg(&input).arg(&wav).status().await?;
    let _ = std::fs::remove_file(&input);
    if !st.success() {
        return Err(anyhow!("couldn't read that audio file").into());
    }
    let base = wav.with_extension("");
    let mut cmd = Command::new(engine_bin(Engine::WhisperCli)?);
    cmd.arg("-m").arg(&m.weights).arg("-f").arg(&wav).arg("-otxt").arg("-of").arg(&base).arg("-nt").arg("-np")
        .args(["-t", &s.effective_threads().to_string()]);
    if !s.use_gpu {
        cmd.arg("-ng");
    }
    let r = core.engines.run_job(&m, "transcribe", cmd).await;
    let _ = std::fs::remove_file(&wav);
    r?;
    let txt = base.with_extension("txt");
    let text = std::fs::read_to_string(&txt).unwrap_or_default().trim().to_string();
    let _ = std::fs::remove_file(&txt);
    Ok(Json(json!({ "text": text })))
}

#[derive(Deserialize)]
struct SpeakReq {
    input: String,
    model: Option<String>,
}

async fn speak(State(core): State<Arc<Core>>, Json(req): Json<SpeakReq>) -> R<Response> {
    if req.input.trim().is_empty() {
        return Err(anyhow!("input is empty").into());
    }
    let m = resolve(&core, req.model.as_deref(), Kind::Speak).await?;
    let vocoder = m.vocoder.clone().ok_or_else(|| anyhow!("{} has no vocoder file", m.name))?;
    let s = settings(&core).await;
    let out = out_path("wav");
    let mut cmd = Command::new(engine_bin(Engine::LlamaTts)?);
    cmd.arg("-m").arg(&m.weights).arg("-mv").arg(&vocoder).arg("-p").arg(&req.input).arg("-o").arg(&out)
        .args(["-t", &s.effective_threads().to_string()])
        .args(["-ngl", if s.use_gpu { "999" } else { "0" }]);
    core.engines.run_job(&m, "speak", cmd).await?;
    let bytes = std::fs::read(&out)?;
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
    if let Ok(v) = HeaderValue::from_str(&out.to_string_lossy()) {
        resp.headers_mut().insert("x-output-path", v);
    }
    Ok(resp)
}

#[derive(Deserialize)]
struct ImageReq {
    prompt: String,
    model: Option<String>,
    size: Option<String>,
    steps: Option<u32>,
    seed: Option<i64>,
    negative_prompt: Option<String>,
}

async fn image(State(core): State<Arc<Core>>, Json(req): Json<ImageReq>) -> R<Json<Value>> {
    if req.prompt.trim().is_empty() {
        return Err(anyhow!("prompt is empty").into());
    }
    let m = resolve(&core, req.model.as_deref(), Kind::Image).await?;
    let s = settings(&core).await;
    let (w, h) = req
        .size
        .as_deref()
        .and_then(|sz| sz.split_once('x'))
        .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
        .unwrap_or((512, 512));
    let (w, h) = (w.clamp(256, 1024) / 64 * 64, h.clamp(256, 1024) / 64 * 64);
    let turbo = m.id.contains("turbo") || m.name.to_lowercase().contains("turbo");
    let steps = req.steps.unwrap_or(if turbo { 4 } else { 20 }).clamp(1, 60);
    let cfg = if turbo { "1.0" } else { "7.0" };
    let out = out_path("png");
    let mut cmd = Command::new(engine_bin(Engine::SdCli)?);
    cmd.arg("-m").arg(&m.weights).arg("-p").arg(&req.prompt).arg("-o").arg(&out)
        .args(["-W", &w.to_string(), "-H", &h.to_string(), "--steps", &steps.to_string(), "--cfg-scale", cfg])
        .args(["-t", &s.effective_threads().to_string()])
        .args(["-s", &req.seed.unwrap_or(-1).to_string()]);
    if let Some(n) = req.negative_prompt.as_deref().filter(|n| !n.is_empty()) {
        cmd.arg("-n").arg(n);
    }
    core.engines.run_job(&m, "image", cmd).await?;
    let bytes = std::fs::read(&out)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let created = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    Ok(Json(json!({ "created": created, "data": [{ "b64_json": b64, "path": out }] })))
}

#[derive(Deserialize)]
struct VideoReq {
    prompt: String,
    model: Option<String>,
    size: Option<String>,
    seconds: Option<f64>,
    steps: Option<u32>,
    seed: Option<i64>,
    negative_prompt: Option<String>,
}

/// Text to video with a Wan model. Not part of the OpenAI API; it mirrors the images endpoint and
/// returns the clip's path (WebM) and its base64.
async fn video(State(core): State<Arc<Core>>, Json(req): Json<VideoReq>) -> R<Json<Value>> {
    if req.prompt.trim().is_empty() {
        return Err(anyhow!("prompt is empty").into());
    }
    let m = resolve(&core, req.model.as_deref(), Kind::Video).await?;
    let (Some(t5), Some(vae)) = (m.text_encoder.clone(), m.vae.clone()) else {
        return Err(anyhow!("{} needs its umt5 text encoder and VAE files in its folder", m.name).into());
    };
    let s = settings(&core).await;
    let (w, h) = req
        .size
        .as_deref()
        .and_then(|sz| sz.split_once('x'))
        .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
        .unwrap_or((480, 272));
    let (w, h) = (w.clamp(256, 832) / 16 * 16, h.clamp(256, 832) / 16 * 16);
    // Wan makes 4n+1 frames at 16 fps
    let fps = 16.0;
    let frames = (((req.seconds.unwrap_or(2.0).clamp(1.0, 5.0) * fps) as u32) / 4 * 4 + 1).max(5);
    let steps = req.steps.unwrap_or(15).clamp(4, 50);
    let out = out_path("webm");
    let mut cmd = Command::new(engine_bin(Engine::SdCli)?);
    cmd.args(["-M", "vid_gen"]).arg("--diffusion-model").arg(&m.weights).arg("--t5xxl").arg(&t5).arg("--vae").arg(&vae)
        .arg("-p").arg(&req.prompt).arg("-o").arg(&out)
        .args(["-W", &w.to_string(), "-H", &h.to_string(), "--video-frames", &frames.to_string(), "--fps", "16"])
        .args(["--steps", &steps.to_string(), "--cfg-scale", "6.0", "--sampling-method", "euler", "--flow-shift", "3.0"])
        .args(["--vae-tiling", "-t", &s.effective_threads().to_string(), "-s", &req.seed.unwrap_or(-1).to_string()]);
    if let Some(n) = req.negative_prompt.as_deref().filter(|n| !n.is_empty()) {
        cmd.arg("-n").arg(n);
    }
    core.engines.run_job(&m, "video", cmd).await?;
    let bytes = std::fs::read(&out)?;
    let created = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    Ok(Json(json!({ "created": created, "data": [{ "b64_webm": base64::engine::general_purpose::STANDARD.encode(bytes), "path": out, "frames": frames, "size": format!("{w}x{h}") }] })))
}

/// How well a model fits: "great", "ok" (inside the budget), "over_budget" (the Mac could, the budget
/// says no) or "too_big" (more than this Mac has).
fn fit(need: f64, s: &Settings) -> &'static str {
    let ram = system::total_memory_gb();
    if need <= s.memory_budget_gb * 0.75 {
        "great"
    } else if need <= s.memory_budget_gb {
        "ok"
    } else if need <= ram - 3.0 {
        "over_budget"
    } else {
        "too_big"
    }
}

async fn status(State(core): State<Arc<Core>>) -> Json<Value> {
    let s = settings(&core).await;
    let engines: Vec<Value> = Engine::ALL.iter().map(|e| json!({ "engine": e, "path": e.path() })).collect();
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "system": {
            "ram_total_gb": system::total_memory_gb(),
            "ram_available_gb": system::available_memory_gb(),
            "disk_free_gb": system::free_disk_gb(&s.models_dir),
            "cpus": system::cpu_count(),
        },
        "settings": s,
        "running": core.engines.running().await,
        "max_loaded": s.effective_max_loaded(),
        "job": core.engines.current_job(),
        "used_gb": core.engines.used_gb().await,
        "events": core.engines.events(),
        "engines": engines,
        "downloads": core.downloads.all(),
        "models_move": core.mover.progress(),
        "models_drive_error": s.models_drive_ready().err().map(|e| e.to_string()),
        "api_url": format!("http://127.0.0.1:{}/v1", s.api_port),
    }))
}

async fn library_list(State(core): State<Arc<Core>>) -> Json<Value> {
    let s = settings(&core).await;
    let list: Vec<Value> = library::scan(&s)
        .into_iter()
        .map(|m| {
            let need = estimate_gb(&m, &s);
            let mut v = serde_json::to_value(&m).unwrap_or_default();
            v["need_gb"] = json!(need);
            v["fit"] = json!(fit(need, &s));
            v["attribution"] = match catalog::get(&m.id) {
                Some(c) => json!(catalog::attribution(&c)),
                None => imported_source(&m.dir),
            };
            v
        })
        .collect();
    Json(json!(list))
}

/// For a model added from a link: the Hugging Face repo (or site) it was downloaded from.
fn imported_source(dir: &std::path::Path) -> Value {
    let Some(url) = std::fs::read(dir.join("source.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["urls"][0].as_str().map(str::to_string))
    else {
        return Value::Null;
    };
    let repo = url
        .strip_prefix("https://huggingface.co/")
        .map(|r| r.split('/').take(2).collect::<Vec<_>>().join("/"))
        .unwrap_or_else(|| url.split('/').nth(2).unwrap_or("").to_string());
    json!({ "license": "See the model page", "license_url": url, "origin": repo, "download_repo": repo, "imported_from": url })
}

async fn catalog_list(State(core): State<Arc<Core>>) -> Json<Value> {
    let s = settings(&core).await;
    let installed: Vec<String> = library::scan(&s).into_iter().map(|m| m.id).collect();
    let list: Vec<Value> = catalog::all()
        .into_iter()
        .map(|m| {
            let size = m.size_gb();
            let need = match m.kind {
                Kind::Chat => size + s.context_size as f64 * 7.5e-5 * (size / 5.0).clamp(0.2, 3.0) + 0.9,
                Kind::Image => size * 1.3 + 1.2,
                Kind::Video => size * 1.2 + 1.5,
                Kind::Speak => size + 0.6,
                Kind::Transcribe => size * 1.5 + 0.3,
            };
            let mut v = serde_json::to_value(&m).unwrap_or_default();
            v["size_gb"] = json!(size);
            v["need_gb"] = json!(need);
            v["fit"] = json!(fit(need, &s));
            v["installed"] = json!(installed.iter().any(|i| i == m.id));
            v["attribution"] = json!(catalog::attribution(&m));
            v
        })
        .collect();
    Json(json!(list))
}

#[derive(Deserialize)]
struct IdReq {
    id: String,
}

#[derive(Deserialize)]
struct LaunchReq {
    id: String,
    /// Load even past the RAM budget and free memory ("Run anyway").
    #[serde(default)]
    force: bool,
}

async fn launch(State(core): State<Arc<Core>>, Json(req): Json<LaunchReq>) -> R<Json<Value>> {
    let m = resolve(&core, Some(&req.id), Kind::Chat).await?;
    let port = core.engines.load_chat(&m, req.force).await?;
    Ok(Json(json!({ "id": m.id, "name": m.name, "port": port, "url": format!("http://127.0.0.1:{port}/") })))
}

#[derive(Deserialize, Default)]
struct StopReq {
    id: Option<String>,
}

/// Unloads one chat model (`{"id": ...}`), or all of them.
async fn stop(State(core): State<Arc<Core>>, body: Option<Json<StopReq>>) -> Json<Value> {
    match body.and_then(|Json(b)| b.id) {
        Some(id) => core.engines.stop_model(&id, "you unloaded it").await,
        None => core.engines.stop_chat("you unloaded it").await,
    }
    Json(json!({ "ok": true }))
}

async fn download_start(State(core): State<Arc<Core>>, Json(req): Json<IdReq>) -> R<Json<Value>> {
    core.downloads.start(&req.id, core.settings.clone())?;
    Ok(Json(json!({ "ok": true })))
}

fn kind_from(k: &str) -> anyhow::Result<Kind> {
    Ok(match k {
        "chat" => Kind::Chat,
        "transcribe" | "speak" | "voice" => Kind::Transcribe,
        "image" => Kind::Image,
        "video" => Kind::Video,
        _ => anyhow::bail!("kind must be chat, voice, image or video"),
    })
}

#[derive(Deserialize)]
struct ImportUrlReq {
    name: String,
    kind: String,
    urls: Vec<String>,
}

/// Downloads a model from links into ~/Models/<Kind>/<name>/.
async fn import_url(State(core): State<Arc<Core>>, Json(req): Json<ImportUrlReq>) -> R<Json<Value>> {
    let s = settings(&core).await;
    s.models_drive_ready()?;
    let id = download::folder_name(&req.name);
    let dir = s.kind_dir(kind_from(&req.kind)?).join(&id);
    core.downloads.start_urls(&id, dir.clone(), req.urls.clone())?;
    // remember where it came from, so the card can credit its source
    let _ = std::fs::write(dir.join("source.json"), serde_json::to_vec_pretty(&json!({ "urls": req.urls }))?);
    core.engines.note(format!("Downloading {id}"));
    Ok(Json(json!({ "ok": true, "id": id })))
}

#[derive(Deserialize)]
struct ImportFileReq {
    name: String,
    kind: String,
    paths: Vec<String>,
}

/// Moves model files that are already on this Mac into ~/Models/<Kind>/<name>/, so they're kept in
/// one place without a second copy eating disk. Across disks it copies instead.
async fn import_file(State(core): State<Arc<Core>>, Json(req): Json<ImportFileReq>) -> R<Json<Value>> {
    let s = settings(&core).await;
    s.models_drive_ready()?;
    let id = download::folder_name(&req.name);
    let dir = s.kind_dir(kind_from(&req.kind)?).join(&id);
    std::fs::create_dir_all(&dir)?;
    let mut moved = 0;
    for p in &req.paths {
        let src = PathBuf::from(p);
        let n = src.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !src.is_file() || !(n.ends_with(".gguf") || n.ends_with(".bin") || n.ends_with(".safetensors")) {
            return Err(anyhow!("{p} isn't a .gguf, .bin or .safetensors file").into());
        }
        let dest = dir.join(src.file_name().unwrap());
        if std::fs::rename(&src, &dest).is_err() {
            std::fs::copy(&src, &dest)?;
        }
        moved += 1;
    }
    core.engines.note(format!("Added {id} ({moved} file{})", if moved == 1 { "" } else { "s" }));
    Ok(Json(json!({ "ok": true, "id": id })))
}

async fn download_cancel(State(core): State<Arc<Core>>, Json(req): Json<IdReq>) -> Json<Value> {
    core.downloads.cancel(&req.id);
    Json(json!({ "ok": true }))
}

async fn delete(State(core): State<Arc<Core>>, Json(req): Json<IdReq>) -> R<Json<Value>> {
    let s = settings(&core).await;
    let m = library::find(&s, &req.id).ok_or_else(|| anyhow!("{} isn't installed", req.id))?;
    core.engines.stop_model(&m.id, "it's being deleted").await;
    // only ever a model's own folder inside the models folder
    let root = s.models_dir.canonicalize()?;
    let dir = m.dir.canonicalize()?;
    if !dir.starts_with(&root) || dir.parent().and_then(|p| p.parent()) != Some(root.as_path()) {
        bail_api("refusing to delete something outside the models folder")?;
    }
    std::fs::remove_dir_all(&dir)?;
    core.engines.note(format!("Deleted {} ({:.1} GB freed)", m.name, m.size_gb));
    Ok(Json(json!({ "ok": true, "freed_gb": m.size_gb })))
}

fn bail_api(msg: &str) -> R<()> {
    Err(ApiError(StatusCode::FORBIDDEN, msg.into()))
}

async fn update_settings(State(core): State<Arc<Core>>, Json(patch): Json<Value>) -> R<Json<Value>> {
    let new = save_settings(&core, patch).await?;
    Ok(Json(serde_json::to_value(new)?))
}

async fn save_settings(core: &Arc<Core>, patch: Value) -> anyhow::Result<Settings> {
    let mut s = core.settings.write().await;
    let was_online = s.web_access;
    let mut v = serde_json::to_value(&*s)?;
    if let (Some(obj), Some(p)) = (v.as_object_mut(), patch.as_object()) {
        for (k, val) in p {
            if obj.contains_key(k) {
                obj.insert(k.clone(), val.clone());
            }
        }
    }
    let new: Settings = serde_json::from_value::<Settings>(v)?.clamped();
    new.save()?;
    *s = new.clone();
    drop(s);
    if new.web_access != was_online {
        // Tools are fixed when llama-server starts, so reload loaded models in the background.
        let core = core.clone();
        tokio::spawn(async move { core.engines.reload_chat().await });
    }
    Ok(new)
}

async fn drives(State(core): State<Arc<Core>>) -> Json<Value> {
    let s = settings(&core).await;
    Json(json!({ "models_dir": s.models_dir, "drives": storage::drives(&s.models_dir) }))
}

#[derive(Deserialize)]
struct MoveReq {
    /// A drive's Models folder, as /api/drives lists it.
    path: PathBuf,
}

/// Moves every model to another drive (or back to this Mac). Chat models are unloaded first.
async fn move_models(State(core): State<Arc<Core>>, Json(req): Json<MoveReq>) -> R<Json<Value>> {
    let s = settings(&core).await;
    s.models_drive_ready()?;
    let listed = storage::drives(&s.models_dir).iter().any(|d| d["path"].as_str() == Some(&*req.path.to_string_lossy()));
    if !listed {
        bail_api("pick one of the drives listed in Settings › Storage")?;
    }
    let from = std::fs::canonicalize(&s.models_dir)?;
    let to = req.path.clone();
    if std::fs::canonicalize(&to).is_ok_and(|t| t == from) {
        return Ok(Json(json!({ "ok": true, "moved": false })));
    }
    if to.read_dir().is_ok_and(|mut d| d.any(|e| e.is_ok_and(|e| e.file_name() != ".DS_Store" && e.file_name() != "About this folder.txt"))) && to != storage::home_models() {
        bail_api(&format!("{} already has files in it; move or rename that folder first", to.display()))?;
    }
    if core.downloads.all().values().any(|d| !d.finished && d.error.is_none()) {
        bail_api("wait for downloads to finish first")?;
    }
    core.engines.stop_chat("its models are moving to another drive").await;
    storage::relink_home(&storage::home_models()); // ~/Models becomes a real folder for the move back
    std::fs::create_dir_all(&to)?;
    let c = core.clone();
    let rt = tokio::runtime::Handle::current();
    core.mover.start(from, to, move |dest| {
        rt.block_on(async {
            let _ = save_settings(&c, json!({ "models_dir": dest })).await;
            storage::relink_home(&dest);
            let s = settings(&c).await;
            library::tidy(&s);
            c.engines.note(format!("Models moved to {}", dest.display()));
        });
    })?;
    Ok(Json(json!({ "ok": true, "moved": true })))
}

#[derive(Deserialize)]
struct SearchReq {
    query: String,
    max_results: Option<usize>,
}

#[derive(Deserialize)]
struct FetchReq {
    url: String,
}

async fn web_search(State(core): State<Arc<Core>>, Json(req): Json<SearchReq>) -> R<Json<Value>> {
    if !settings(&core).await.web_access {
        bail_api("Internet access is off in Settings")?;
    }
    let results = web::search(&req.query, req.max_results.unwrap_or(6).clamp(1, 10)).await?;
    Ok(Json(json!({ "query": req.query, "results": results })))
}

async fn web_fetch(State(core): State<Arc<Core>>, Json(req): Json<FetchReq>) -> R<Json<Value>> {
    if !settings(&core).await.web_access {
        bail_api("Internet access is off in Settings")?;
    }
    Ok(Json(web::fetch(&req.url).await?))
}

#[derive(Deserialize)]
struct EngineReq {
    engine: Engine,
}

async fn install_engine(Json(req): Json<EngineReq>) -> R<Json<Value>> {
    let msg = download::install_engine(req.engine).await?;
    Ok(Json(json!({ "ok": true, "message": msg })))
}

#[derive(Deserialize)]
struct RevealReq {
    id: Option<String>,
    path: Option<String>,
}

async fn reveal(State(core): State<Arc<Core>>, Json(req): Json<RevealReq>) -> R<Json<Value>> {
    let s = settings(&core).await;
    let target = match (req.id, req.path) {
        (Some(id), _) => library::find(&s, &id).map(|m| m.dir).ok_or_else(|| anyhow!("{id} isn't installed"))?,
        (_, Some(p)) => PathBuf::from(p),
        _ => s.models_dir.clone(),
    };
    Command::new("/usr/bin/open").arg("-R").arg(&target).status().await?;
    Ok(Json(json!({ "ok": true })))
}
