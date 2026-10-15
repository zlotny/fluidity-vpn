use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use fluidity_ipc::{Event, LogEntry, Prompt, Reply, SessionInfo, SystemSettings};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc};

use crate::killswitch::{Lock, Rules};
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
    pub kill_switch: Mutex<BTreeMap<String, Lock>>,
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
            kill_switch: Mutex::default(),
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
        let result = match (before, *holds) {
            (0, 1) => self.backend.pause_time_machine(),
            (1, 0) => self.backend.resume_time_machine(),
            _ => Ok(()),
        };
        if let Err(err) = result {
            tracing::warn!("Time Machine: {err}");
        }
    }

    pub async fn engage_kill_switch(&self, id: &str, lock: Lock) -> Result<(), String> {
        let mut locks = self.kill_switch.lock().await;
        let owner = lock.owner;
        locks.insert(id.to_string(), lock);
        let result = self.apply_kill_switch(&locks);
        self.announce_blocked(&locks, owner);
        result
    }

    pub async fn kill_switch_connected(&self, id: &str, connected: bool) {
        let mut locks = self.kill_switch.lock().await;
        if let Some(lock) = locks.get_mut(id).filter(|l| l.connected != connected) {
            lock.connected = connected;
            if let Err(err) = self.apply_kill_switch(&locks) {
                tracing::warn!("kill switch: {err}");
            }
        }
    }

    pub async fn orphan_kill_switch(&self, id: &str) {
        let mut locks = self.kill_switch.lock().await;
        let Some(lock) = locks.get_mut(id) else { return };
        lock.orphaned = true;
        lock.connected = false;
        let owner = lock.owner;
        if let Err(err) = self.apply_kill_switch(&locks) {
            tracing::warn!("kill switch: {err}");
        }
        self.announce_blocked(&locks, owner);
    }

    pub async fn adopt_kill_switch(&self, id: &str) {
        let mut locks = self.kill_switch.lock().await;
        let Some(lock) = locks.get_mut(id) else { return };
        lock.orphaned = false;
        let owner = lock.owner;
        self.announce_blocked(&locks, owner);
    }

    pub async fn release_kill_switch(&self, uid: u32, id: Option<&str>) {
        let mut locks = self.kill_switch.lock().await;
        let before = locks.len();
        locks.retain(|key, lock| {
            let owned = uid == crate::server::ROOT || lock.owner == uid;
            let matches = match id {
                Some(id) => key == id,
                None => lock.orphaned,
            };
            !(owned && matches)
        });
        if locks.len() == before {
            return;
        }
        if let Err(err) = self.apply_kill_switch(&locks) {
            tracing::warn!("kill switch: {err}");
        }
        self.announce_blocked(&locks, uid);
    }

    pub async fn blocked(&self, uid: u32) -> Vec<String> {
        let locks = self.kill_switch.lock().await;
        blocked_ids(&locks, uid)
    }

    fn apply_kill_switch(&self, locks: &BTreeMap<String, Lock>) -> Result<(), String> {
        self.backend.kill_switch(Rules::from_locks(locks).as_ref())
    }

    fn announce_blocked(&self, locks: &BTreeMap<String, Lock>, uid: u32) {
        let _ = self.events.send((
            uid,
            Event::Blocked {
                ids: blocked_ids(locks, uid),
            },
        ));
    }
}

fn blocked_ids(locks: &BTreeMap<String, Lock>, uid: u32) -> Vec<String> {
    locks
        .iter()
        .filter(|(_, l)| l.orphaned && (uid == crate::server::ROOT || l.owner == uid))
        .map(|(id, _)| id.clone())
        .collect()
}
