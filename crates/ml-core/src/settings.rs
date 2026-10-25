//! User settings, including the resource limits every engine runs under.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::system;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where models live: one folder per kind (Chat, Voice, Image), one subfolder per model.
    pub models_dir: PathBuf,
    /// The most memory all running models may use together. Launches that wouldn't fit are refused,
    /// and the watchdog stops a model that grows past it.
    pub memory_budget_gb: f64,
    /// CPU threads per engine; 0 picks a sensible number for this Mac.
    pub threads: u32,
    /// Run model layers on the GPU (Metal). Off means CPU only: slower, but leaves the GPU free.
    pub use_gpu: bool,
    /// How many tokens a chat model keeps in memory. Bigger remembers more and costs more memory.
    pub context_size: u32,
    /// Unload a chat model after this many idle minutes; 0 keeps it loaded.
    pub idle_unload_minutes: u32,
    /// Run engines at low CPU priority so the rest of the Mac stays responsive.
    pub low_priority: bool,
    /// Port for the local OpenAI-compatible API (and the app's own UI).
    pub api_port: u16,
    /// Chat models that may be loaded at the same time; 0 picks by this Mac's RAM. They still
    /// have to fit the RAM budget together.
    pub max_loaded: u32,
    /// Let chat models search the web and read pages. Off keeps them fully offline.
    pub web_access: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let ram = system::total_memory_gb();
        Settings {
            models_dir: dirs::home_dir().unwrap_or_default().join("Models"),
            memory_budget_gb: (ram * 0.5).round().max(2.0),
            threads: 0,
            use_gpu: true,
            context_size: 8192,
            idle_unload_minutes: 10,
            low_priority: true,
            api_port: 11500,
            max_loaded: 0,
            web_access: true,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        support_dir().join("settings.json")
    }

    pub fn load() -> Settings {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .unwrap_or_default()
            .clamped()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(support_dir())?;
        std::fs::write(Self::path(), serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    /// Keeps values inside what this Mac can actually give.
    pub fn clamped(mut self) -> Settings {
        let ram = system::total_memory_gb();
        self.memory_budget_gb = self.memory_budget_gb.clamp(1.0, (ram - 2.0).max(1.0));
        self.threads = self.threads.min(system::cpu_count() as u32);
        self.context_size = self.context_size.clamp(512, 131_072);
        self.max_loaded = self.max_loaded.min(8);
        self
    }

    pub fn effective_threads(&self) -> u32 {
        if self.threads > 0 {
            self.threads
        } else {
            (system::cpu_count() as u32).saturating_sub(2).max(2)
        }
    }

    pub fn effective_max_loaded(&self) -> u32 {
        if self.max_loaded > 0 {
            return self.max_loaded;
        }
        match system::total_memory_gb() {
            r if r >= 48.0 => 4,
            r if r >= 32.0 => 3,
            r if r >= 24.0 => 2,
            _ => 1,
        }
    }

    pub fn kind_dir(&self, kind: crate::catalog::Kind) -> PathBuf {
        self.models_dir.join(kind.folder())
    }
}

/// ~/Library/Application Support/Model Launcher: settings, engines, logs and outputs (never models).
pub fn support_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_default().join("Model Launcher")
}

pub fn outputs_dir() -> PathBuf {
    support_dir().join("outputs")
}

pub fn logs_dir() -> PathBuf {
    support_dir().join("logs")
}
