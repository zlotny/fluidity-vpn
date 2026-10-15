use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::preferences::Preferences;
use crate::store::{Error, Store};

pub const MANIFEST: &str = "deploy.json";
const CONNECTIONS: &str = "Connections";
const MENU_ITEMS: &str = "Menu Items";
const PREFERENCES: &str = "preferences.json";
const APPLIED: &str = "deployed.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    Append,
    #[default]
    Replace,
    Overwrite,
    Prompt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Manifest {
    pub version: String,
    pub connections: ConnectionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Deployment {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub connections: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct Report {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

pub fn system_dirs() -> Vec<PathBuf> {
    if let Some(dir) = std::env::var_os("FLUIDITY_DEPLOY_DIR") {
        return vec![dir.into()];
    }
    if cfg!(target_os = "macos") {
        vec!["/Library/Application Support/Fluidity/Deploy".into()]
    } else if cfg!(windows) {
        vec![r"C:\ProgramData\Fluidity\Deploy".into()]
    } else {
        vec!["/etc/fluidity/deploy".into()]
    }
}

fn applied(store: &Store) -> BTreeMap<String, String> {
    fs::read(store.root().join(APPLIED))
        .ok()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default()
}

fn key(dir: &Path) -> String {
    dir.to_string_lossy().into_owned()
}

impl Deployment {
    pub fn open(dir: &Path) -> Option<Self> {
        let manifest: Manifest = match fs::read(dir.join(MANIFEST)) {
            Ok(data) => serde_json::from_slice(&data).ok()?,
            Err(_) => return None,
        };
        let mut connections: Vec<PathBuf> = fs::read_dir(dir.join(CONNECTIONS))
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| is_connection(p))
                    .collect()
            })
            .unwrap_or_default();
        connections.sort();
        Some(Self {
            dir: dir.to_path_buf(),
            manifest,
            connections,
        })
    }

    pub fn pending(store: &Store, dirs: &[PathBuf]) -> Option<Self> {
        let done = applied(store);
        dirs.iter()
            .filter_map(|d| Self::open(d))
            .find(|d| done.get(&key(&d.dir)) != Some(&d.manifest.version))
    }

    pub fn apply(&self, store: &Store, mode: ConnectionMode) -> Result<Report, Error> {
        let mut report = Report::default();
        if mode == ConnectionMode::Overwrite {
            for conn in store.connections() {
                store.remove(&conn.id)?;
                report.removed.push(conn.name);
            }
        }
        for path in &self.connections {
            let bundle = match fluidity_config::Bundle::import(path) {
                Ok(b) => b,
                Err(e) => {
                    report.errors.push(format!("{}: {e}", path.display()));
                    continue;
                }
            };
            let fallback = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Imported")
                .to_string();
            let name = bundle.name().map(str::to_string).unwrap_or_else(|| fallback.clone());
            match store.find(&name) {
                Ok(existing) if mode == ConnectionMode::Append => {
                    report.skipped.push(existing.name);
                }
                Ok(existing) => {
                    store.save(&existing.id, &bundle)?;
                    report.updated.push(existing.name);
                }
                Err(_) => {
                    let conn = store.add(bundle, &fallback)?;
                    report.added.push(conn.name);
                }
            }
        }
        if let Ok(data) = fs::read(self.dir.join(PREFERENCES))
            && let Ok(overrides) = serde_json::from_slice::<serde_json::Value>(&data)
        {
            let mut prefs = Preferences::load(store.root());
            prefs.merge(&overrides);
            prefs.save(store.root()).map_err(|source| Error::Io {
                path: store.root().to_path_buf(),
                source,
            })?;
        }
        copy_tree(&self.dir.join(MENU_ITEMS), &store.root().join(MENU_ITEMS)).map_err(|source| Error::Io {
            path: self.dir.join(MENU_ITEMS),
            source,
        })?;
        self.mark(store)?;
        Ok(report)
    }

    pub fn mark(&self, store: &Store) -> Result<(), Error> {
        let mut done = applied(store);
        done.insert(key(&self.dir), self.manifest.version.clone());
        let path = store.root().join(APPLIED);
        let data = serde_json::to_vec_pretty(&done).unwrap_or_default();
        fs::write(&path, data).map_err(|source| Error::Io { path, source })
    }
}

fn is_connection(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    !name.starts_with('.')
        && [".ovpn", ".conf", ".fvpn", ".fvpnz", ".tblk", ".zip", ".tar.gz", ".tgz"]
            .iter()
            .any(|ext| name.ends_with(ext))
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conf(name: &str, remote: &str) -> String {
        format!("#fluidity name \"{name}\"\nclient\ndev tun\nremote {remote} 1194\n")
    }

    fn setup(manifest: &str) -> (tempfile::TempDir, tempfile::TempDir) {
        let deploy = tempfile::tempdir().unwrap();
        fs::write(deploy.path().join(MANIFEST), manifest).unwrap();
        fs::create_dir_all(deploy.path().join(CONNECTIONS)).unwrap();
        fs::write(
            deploy.path().join(CONNECTIONS).join("office.ovpn"),
            conf("Office", "new.example"),
        )
        .unwrap();
        fs::write(
            deploy.path().join(CONNECTIONS).join("lab.ovpn"),
            conf("Lab", "lab.example"),
        )
        .unwrap();
        fs::write(deploy.path().join(CONNECTIONS).join("ca.crt"), "x").unwrap();
        fs::create_dir_all(deploy.path().join(MENU_ITEMS)).unwrap();
        fs::write(
            deploy.path().join(MENU_ITEMS).join("Help.fluiditymenuitem"),
            "#!/bin/sh\n",
        )
        .unwrap();
        fs::write(deploy.path().join(PREFERENCES), r#"{"block_ipv6": false}"#).unwrap();
        (deploy, tempfile::tempdir().unwrap())
    }

    fn bundle(text: &str) -> Result<fluidity_config::Bundle, ()> {
        Ok(fluidity_config::Bundle {
            document: fluidity_config::Document::parse(text).unwrap(),
            files: Default::default(),
        })
    }

    fn remote(store: &Store, name: &str) -> String {
        let id = store.find(name).unwrap().id;
        store.load(&id).unwrap().document.directive("remote").unwrap()[0].clone()
    }

    #[test]
    fn applies_once_per_version() {
        let (deploy, home) = setup(r#"{"version": "1"}"#);
        let store = Store::open(home.path()).unwrap();
        store.add(bundle(&conf("Office", "old.example")).unwrap(), "x").unwrap();
        let dirs = [deploy.path().to_path_buf()];
        let pending = Deployment::pending(&store, &dirs).unwrap();
        assert_eq!(pending.manifest.connections, ConnectionMode::Replace);
        assert_eq!(pending.connections.len(), 2);
        let report = pending.apply(&store, ConnectionMode::Replace).unwrap();
        assert_eq!(report.updated, ["Office"]);
        assert_eq!(report.added, ["Lab"]);
        assert_eq!(remote(&store, "Office"), "new.example");
        assert!(!Preferences::load(home.path()).block_ipv6);
        assert!(home.path().join(MENU_ITEMS).join("Help.fluiditymenuitem").is_file());
        assert!(Deployment::pending(&store, &dirs).is_none());
        fs::write(deploy.path().join(MANIFEST), r#"{"version": "2"}"#).unwrap();
        assert!(Deployment::pending(&store, &dirs).is_some());
    }

    #[test]
    fn append_keeps_and_overwrite_clears() {
        let (deploy, home) = setup(r#"{"connections": "append"}"#);
        let store = Store::open(home.path()).unwrap();
        store.add(bundle(&conf("Office", "old.example")).unwrap(), "x").unwrap();
        store.add(bundle(&conf("Mine", "me.example")).unwrap(), "x").unwrap();
        let dep = Deployment::open(deploy.path()).unwrap();
        let report = dep.apply(&store, ConnectionMode::Append).unwrap();
        assert_eq!(report.skipped, ["Office"]);
        assert_eq!(remote(&store, "Office"), "old.example");
        let report = dep.apply(&store, ConnectionMode::Overwrite).unwrap();
        assert_eq!(report.removed.len(), 3);
        assert_eq!(report.added, ["Lab", "Office"]);
        assert!(store.find("Mine").is_err());
    }
}
