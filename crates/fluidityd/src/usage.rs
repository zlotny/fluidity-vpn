use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fluidity_ipc::UsageSample;
use serde::{Deserialize, Serialize};

const HOUR: u64 = 3600;
const KEEP: u64 = 32 * 24 * HOUR;
const SAVE_EVERY: u64 = 60;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    connections: BTreeMap<String, BTreeMap<u64, (u64, u64)>>,
}

pub struct Usage {
    file: PathBuf,
    stored: Stored,
    saved_at: u64,
    dirty: bool,
}

fn key(uid: u32, id: &str) -> String {
    format!("{uid}:{id}")
}

impl Usage {
    pub fn load(file: PathBuf) -> Self {
        let stored = std::fs::read(&file)
            .ok()
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default();
        Self {
            file,
            stored,
            saved_at: 0,
            dirty: false,
        }
    }

    pub fn add(&mut self, uid: u32, id: &str, now: u64, bytes_in: u64, bytes_out: u64) {
        if bytes_in == 0 && bytes_out == 0 {
            return;
        }
        let hours = self.stored.connections.entry(key(uid, id)).or_default();
        let slot = hours.entry(now - now % HOUR).or_default();
        slot.0 += bytes_in;
        slot.1 += bytes_out;
        self.dirty = true;
        if now >= self.saved_at + SAVE_EVERY {
            self.save(now);
        }
    }

    pub fn history(&self, uid: u32, id: &str, now: u64) -> Vec<UsageSample> {
        let since = now.saturating_sub(KEEP);
        self.stored
            .connections
            .get(&key(uid, id))
            .map(|hours| {
                hours
                    .range(since..)
                    .map(|(&hour, &(bytes_in, bytes_out))| UsageSample {
                        hour,
                        bytes_in,
                        bytes_out,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn clear(&mut self, uid: u32, id: &str, now: u64) {
        if self.stored.connections.remove(&key(uid, id)).is_some() {
            self.dirty = true;
            self.save(now);
        }
    }

    pub fn save(&mut self, now: u64) {
        if !self.dirty {
            return;
        }
        let since = now.saturating_sub(KEEP);
        for hours in self.stored.connections.values_mut() {
            hours.retain(|&hour, _| hour >= since);
        }
        self.stored.connections.retain(|_, hours| !hours.is_empty());
        if write(&self.file, &self.stored).is_ok() {
            self.dirty = false;
        }
        self.saved_at = now;
    }
}

fn write(file: &Path, stored: &Stored) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(stored)?)?;
    std::fs::rename(tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_hourly_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("usage.json");
        let mut usage = Usage::load(file.clone());
        let day = 24 * HOUR;
        usage.add(501, "a", 40 * day + 10, 100, 10);
        usage.add(501, "a", 40 * day + 20, 5, 1);
        usage.add(501, "a", 40 * day + HOUR + 1, 7, 0);
        usage.add(502, "a", 40 * day, 1, 1);
        usage.add(501, "a", 40 * day + 30, 0, 0);
        let history = usage.history(501, "a", 40 * day + HOUR + 1);
        assert_eq!(history.len(), 2);
        assert_eq!(
            (history[0].hour, history[0].bytes_in, history[0].bytes_out),
            (40 * day, 105, 11)
        );
        assert_eq!(history[1].bytes_in, 7);

        usage.add(501, "a", 80 * day, 1, 2);
        usage.save(80 * day);
        let reloaded = Usage::load(file);
        let history = reloaded.history(501, "a", 80 * day);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].hour, 80 * day);
        assert!(reloaded.history(502, "a", 80 * day).is_empty());

        let mut usage = reloaded;
        usage.clear(501, "a", 80 * day);
        assert!(usage.history(501, "a", 80 * day).is_empty());
    }
}
