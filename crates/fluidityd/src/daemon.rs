use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use fluidity_ipc::{Event, LogEntry, Prompt, Reply, SessionInfo, SystemSettings};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc};

use crate::net::Backend;

pub const LOG_LIMIT: usize = 5000;

#[derive(Debug, Clone)]
pub struct Paths {
    pub run_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn system() -> Self {
        let env = |key: &str| std::env::var_os(key).map(PathBuf::from);
        let (run, data) = if cfg!(target_os = "macos") {
            ("/var/run/fluidity", "/Library/Application Support/Fluidity")
        } else if cfg!(windows) {
            (r"C:\ProgramData\Fluidity\run", r"C:\ProgramData\Fluidity")
        } else {
            ("/run/fluidity", "/var/lib/fluidity")
        };
        Self {
            run_dir: env("FLUIDITY_RUN_DIR").unwrap_or_else(|| run.into()),
            data_dir: env("FLUIDITY_DATA_DIR").unwrap_or_else(|| data.into()),
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.data_dir.join("system.json")
    }

    pub fn net_state_dir(&self) -> PathBuf {
        self.data_dir.join("state")
    }
}

pub enum Command {
    Disconnect,
    Reply(Reply),
}

pub struct Entry {
    pub generation: u64,
    pub owner: u32,
    pub commands: mpsc::UnboundedSender<Command>,
    pub info: SessionInfo,
    pub log: VecDeque<LogEntry>,
    pub prompt: Option<Prompt>,
}

pub struct Daemon {
    pub paths: Paths,
    pub backend: Box<dyn Backend>,
    pub sessions: Mutex<HashMap<String, Entry>>,
    pub events: broadcast::Sender<(u32, Event)>,
    pub settings: RwLock<SystemSettings>,
    pub time_machine_holds: Mutex<usize>,
    pub gui_version: String,
    pub generation: std::sync::atomic::AtomicU64,
}

impl Daemon {
    pub fn new(paths: Paths, backend: Box<dyn Backend>) -> Arc<Self> {
        let settings = std::fs::read(paths.settings_file())
            .ok()
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default();
        Arc::new(Self {
            paths,
            backend,
            sessions: Mutex::default(),
            events: broadcast::channel(4096).0,
            settings: RwLock::new(settings),
            time_machine_holds: Mutex::new(0),
            gui_version: format!("Fluidity_{}", env!("CARGO_PKG_VERSION")),
            generation: std::sync::atomic::AtomicU64::new(1),
        })
    }

    pub async fn save_settings(&self, settings: SystemSettings) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.paths.data_dir)?;
        std::fs::write(self.paths.settings_file(), serde_json::to_vec_pretty(&settings)?)?;
        *self.settings.write().await = settings;
        Ok(())
    }

    pub fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    pub async fn publish(&self, generation: u64, info: &SessionInfo) {
        let mut sessions = self.sessions.lock().await;
        if let Some(entry) = sessions.get_mut(&info.id).filter(|e| e.generation == generation) {
            entry.info = info.clone();
            let _ = self
                .events
                .send((entry.owner, Event::Session { session: info.clone() }));
        }
    }

    pub async fn log(&self, generation: u64, id: &str, entry: LogEntry) {
        tracing::debug!(session = id, "{}", entry.message);
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(id).filter(|e| e.generation == generation) {
            if session.log.len() >= LOG_LIMIT {
                session.log.pop_front();
            }
            session.log.push_back(entry.clone());
            let _ = self.events.send((
                session.owner,
                Event::Log {
                    id: id.to_string(),
                    entry,
                },
            ));
        }
    }

    pub async fn prompt(&self, generation: u64, id: &str, prompt: Option<Prompt>) {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(id).filter(|e| e.generation == generation) {
            session.prompt = prompt.clone();
            let event = match prompt {
                Some(prompt) => Event::Prompt {
                    id: id.to_string(),
                    prompt,
                },
                None => Event::PromptDone { id: id.to_string() },
            };
            let _ = self.events.send((session.owner, event));
        }
    }

    pub async fn remove(&self, generation: u64, id: &str) {
        let mut sessions = self.sessions.lock().await;
        if sessions.get(id).is_some_and(|e| e.generation == generation)
            && let Some(entry) = sessions.remove(id)
        {
            let _ = self.events.send((entry.owner, Event::Removed { id: id.to_string() }));
        }
    }

    pub async fn hold_time_machine(&self, hold: bool) {
        let mut holds = self.time_machine_holds.lock().await;
        let before = *holds;
        *holds = if hold { before + 1 } else { before.saturating_sub(1) };
        let change = match (before, *holds) {
            (0, 1) => Some(false),
            (1, 0) => Some(true),
            _ => None,
        };
        if let Some(enabled) = change
            && let Err(err) = self.backend.set_time_machine(enabled)
        {
            tracing::warn!("Time Machine: {err}");
        }
    }
}
