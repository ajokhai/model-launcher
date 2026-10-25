//! Model Launcher's core: everything except the window. The app embeds it; it can also run headless.

pub mod catalog;
pub mod download;
pub mod engines;
pub mod gateway;
pub mod library;
pub mod settings;
pub mod system;
pub mod web;

use std::sync::Arc;
use tokio::sync::RwLock;

pub use settings::Settings;

pub struct Core {
    pub settings: Arc<RwLock<Settings>>,
    pub engines: Arc<engines::Engines>,
    pub downloads: Arc<download::Downloads>,
}

impl Core {
    pub fn new() -> Arc<Core> {
        let s = Settings::load();
        let moved = library::tidy(&s);
        let settings = Arc::new(RwLock::new(s));
        let engines = engines::Engines::new(settings.clone());
        for m in moved {
            engines.note(format!("Moved {m}"));
        }
        for pid in engines::reap_orphan() {
            engines.note(format!("Stopped a model server left running by the last session (pid {pid})"));
        }
        Arc::new(Core { settings, engines, downloads: Default::default() })
    }

    /// Serves the API and runs the watchdog until the process ends.
    pub async fn serve(self: Arc<Self>) -> anyhow::Result<()> {
        let port = self.settings.read().await.api_port;
        tokio::spawn(self.engines.clone().watchdog());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
        axum::serve(listener, gateway::router(self.clone())).await?;
        Ok(())
    }
}
