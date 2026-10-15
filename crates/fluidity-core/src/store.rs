use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fluidity_config::settings::Settings;
use fluidity_config::{Bundle, Document};
use fluidity_ipc::WireBundle;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] fluidity_config::Error),
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("no connection named \"{0}\"")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub children: Vec<Node>,
    #[serde(default)]
    pub shared_credentials: bool,
    #[serde(default)]
    pub before_connect_script: Option<String>,
    #[serde(default)]
    pub connected_script: Option<String>,
    #[serde(default)]
    pub disconnected_script: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Node {
    Connection(String),
    Folder(Folder),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub dir: PathBuf,
    pub settings: Settings,
    pub folder: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FLUIDITY_HOME") {
        return dir.into();
    }
    let home = || {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/Fluidity")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join("Fluidity")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".config"))
            .join("fluidity")
    }
}

pub fn wire_bundle(bundle: &Bundle) -> WireBundle {
    WireBundle {
        config: bundle.document.to_string(),
        files: bundle
            .files
            .iter()
            .map(|(name, data)| (name.clone(), STANDARD.encode(data)))
            .collect(),
    }
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

impl Store {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, Error> {
        let root = root.into();
        let connections = root.join("connections");
        fs::create_dir_all(&connections).map_err(io(&connections))?;
        Ok(Self { root })
    }

    pub fn open_default() -> Result<Self, Error> {
        Self::open(default_data_dir())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn connections_dir(&self) -> PathBuf {
        self.root.join("connections")
    }

    fn order_file(&self) -> PathBuf {
        self.root.join("order.json")
    }

    pub fn connection_dir(&self, id: &str) -> PathBuf {
        self.connections_dir().join(id)
    }

    fn ids_on_disk(&self) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(self.connections_dir())
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().join(fluidity_config::CONFIG_FILE).is_file())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        ids
    }

    pub fn tree(&self) -> Vec<Node> {
        let stored: Vec<Node> = fs::read(self.order_file())
            .ok()
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default();
        let on_disk: BTreeSet<String> = self.ids_on_disk().into_iter().collect();
        let mut seen = BTreeSet::new();
        let mut tree = prune(stored, &on_disk, &mut seen);
        let mut missing: Vec<Connection> = on_disk
            .iter()
            .filter(|id| !seen.contains(*id))
            .filter_map(|id| self.read_connection(id, None).ok())
            .collect();
        missing.sort_by_key(|c| c.name.to_lowercase());
        tree.extend(missing.into_iter().map(|c| Node::Connection(c.id)));
        tree
    }

    pub fn set_tree(&self, tree: &[Node]) -> Result<(), Error> {
        let path = self.order_file();
        let data = serde_json::to_vec_pretty(tree).map_err(|e| Error::Invalid(e.to_string()))?;
        write_atomic(&path, &data)
    }

    fn read_connection(&self, id: &str, folder: Option<String>) -> Result<Connection, Error> {
        let dir = self.connection_dir(id);
        let path = dir.join(fluidity_config::CONFIG_FILE);
        let text = fs::read_to_string(&path).map_err(io(&path))?;
        let settings = Settings::from_document(&Document::parse(&text)?);
        Ok(Connection {
            id: id.to_string(),
            name: if settings.name.is_empty() {
                id.to_string()
            } else {
                settings.name.clone()
            },
            dir,
            settings,
            folder,
        })
    }

    pub fn connections(&self) -> Vec<Connection> {
        fn walk(store: &Store, nodes: &[Node], folder: Option<&str>, out: &mut Vec<Connection>) {
            for node in nodes {
                match node {
                    Node::Connection(id) => {
                        if let Ok(c) = store.read_connection(id, folder.map(str::to_string)) {
                            out.push(c);
                        }
                    }
                    Node::Folder(f) => walk(store, &f.children, Some(&f.id), out),
                }
            }
        }
        let mut out = Vec::new();
        walk(self, &self.tree(), None, &mut out);
        out
    }

    pub fn folder(&self, id: &str) -> Option<Folder> {
        fn find(nodes: &[Node], id: &str) -> Option<Folder> {
            nodes.iter().find_map(|n| match n {
                Node::Folder(f) if f.id == id => Some(f.clone()),
                Node::Folder(f) => find(&f.children, id),
                Node::Connection(_) => None,
            })
        }
        find(&self.tree(), id)
    }

    pub fn find(&self, query: &str) -> Result<Connection, Error> {
        let all = self.connections();
        all.iter()
            .find(|c| c.id == query)
            .or_else(|| all.iter().find(|c| c.name == query))
            .or_else(|| all.iter().find(|c| c.name.eq_ignore_ascii_case(query)))
            .cloned()
            .ok_or_else(|| Error::NotFound(query.to_string()))
    }

    pub fn load(&self, id: &str) -> Result<Bundle, Error> {
        Ok(Bundle::from_dir(&self.connection_dir(id))?)
    }

    pub fn save(&self, id: &str, bundle: &Bundle) -> Result<(), Error> {
        let dir = self.connection_dir(id);
        let staging = self.connections_dir().join(format!(".{id}.tmp"));
        let _ = fs::remove_dir_all(&staging);
        bundle.write_dir(&staging)?;
        let old = self.connections_dir().join(format!(".{id}.old"));
        let _ = fs::remove_dir_all(&old);
        if dir.exists() {
            fs::rename(&dir, &old).map_err(io(&dir))?;
        }
        fs::rename(&staging, &dir).map_err(io(&dir))?;
        let _ = fs::remove_dir_all(&old);
        Ok(())
    }

    pub fn unique_name(&self, wanted: &str) -> String {
        let wanted = if wanted.trim().is_empty() {
            "New Connection"
        } else {
            wanted.trim()
        };
        let names: BTreeSet<String> = self.connections().into_iter().map(|c| c.name).collect();
        if !names.contains(wanted) {
            return wanted.to_string();
        }
        (2..)
            .map(|n| format!("{wanted} {n}"))
            .find(|n| !names.contains(n))
            .expect("unbounded")
    }

    pub fn add(&self, mut bundle: Bundle, fallback_name: &str) -> Result<Connection, Error> {
        let mut settings = Settings::from_document(&bundle.document);
        let wanted = if settings.name.is_empty() {
            fallback_name.to_string()
        } else {
            settings.name.clone()
        };
        settings.name = self.unique_name(&wanted);
        settings.apply(&mut bundle.document);
        let id = new_id();
        self.save(&id, &bundle)?;
        let mut tree = self.tree();
        if !contains(&tree, &id) {
            tree.push(Node::Connection(id.clone()));
        }
        self.set_tree(&tree)?;
        self.read_connection(&id, None)
    }

    pub fn import(&self, path: &Path) -> Result<Connection, Error> {
        let bundle = Bundle::import(path)?;
        let fallback = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported")
            .to_string();
        self.add(bundle, &fallback)
    }

    pub fn import_replacing(&self, path: &Path, replace: bool) -> Result<Connection, Error> {
        let bundle = Bundle::import(path)?;
        let name = Settings::from_document(&bundle.document).name;
        if replace
            && !name.is_empty()
            && let Ok(existing) = self.find(&name)
        {
            self.save(&existing.id, &bundle)?;
            return self.read_connection(&existing.id, existing.folder);
        }
        let fallback = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported")
            .to_string();
        self.add(bundle, &fallback)
    }

    pub fn duplicate(&self, id: &str) -> Result<Connection, Error> {
        let bundle = self.load(id)?;
        let name = self.read_connection(id, None)?.name;
        let mut settings = Settings::from_document(&bundle.document);
        settings.name = format!("{name} copy");
        let mut bundle = bundle;
        settings.apply(&mut bundle.document);
        self.add(bundle, &name)
    }

    pub fn remove(&self, id: &str) -> Result<(), Error> {
        let dir = self.connection_dir(id);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(io(&dir))?;
        }
        let tree = remove_from(self.tree(), id);
        self.set_tree(&tree)
    }

    pub fn export(&self, id: &str, path: &Path) -> Result<(), Error> {
        let bundle = self.load(id)?;
        let name = self.read_connection(id, None)?.name;
        if path.extension().and_then(|e| e.to_str()) == Some(fluidity_config::BUNDLE_EXTENSION) {
            return Ok(bundle.write_dir(path)?);
        }
        let data = bundle.to_archive(&name)?;
        fs::write(path, data).map_err(io(path))
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<(), Error> {
        let mut bundle = self.load(id)?;
        let mut settings = Settings::from_document(&bundle.document);
        settings.name = name.to_string();
        settings.apply(&mut bundle.document);
        self.save(id, &bundle)
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), Error> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data).map_err(io(&tmp))?;
    fs::rename(&tmp, path).map_err(io(path))
}

fn contains(nodes: &[Node], id: &str) -> bool {
    nodes.iter().any(|n| match n {
        Node::Connection(c) => c == id,
        Node::Folder(f) => contains(&f.children, id),
    })
}

fn remove_from(nodes: Vec<Node>, id: &str) -> Vec<Node> {
    nodes
        .into_iter()
        .filter(|n| !matches!(n, Node::Connection(c) if c == id))
        .map(|n| match n {
            Node::Folder(mut f) => {
                f.children = remove_from(f.children, id);
                Node::Folder(f)
            }
            other => other,
        })
        .collect()
}

fn prune(nodes: Vec<Node>, on_disk: &BTreeSet<String>, seen: &mut BTreeSet<String>) -> Vec<Node> {
    nodes
        .into_iter()
        .filter_map(|n| match n {
            Node::Connection(id) => (on_disk.contains(&id) && seen.insert(id.clone())).then_some(Node::Connection(id)),
            Node::Folder(mut f) => {
                f.children = prune(f.children, on_disk, seen);
                Some(Node::Folder(f))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path().join("data")).unwrap();
        (tmp, store)
    }

    fn ovpn(dir: &Path, name: &str, meta_name: Option<&str>) -> PathBuf {
        let path = dir.join(format!("{name}.ovpn"));
        let mut text = String::new();
        if let Some(n) = meta_name {
            text.push_str(&format!("#fluidity name \"{n}\"\n"));
        }
        text.push_str("client\nremote vpn.example.com 1194\nca ca.crt\n");
        fs::write(dir.join("ca.crt"), "CA").unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn import_list_find_remove() {
        let (tmp, store) = store();
        let a = store.import(&ovpn(tmp.path(), "office", None)).unwrap();
        assert_eq!(a.name, "office");
        let b = store.import(&ovpn(tmp.path(), "x", Some("office"))).unwrap();
        assert_eq!(b.name, "office 2");
        assert_eq!(store.connections().len(), 2);
        assert_eq!(store.find("office 2").unwrap().id, b.id);
        assert_eq!(store.load(&a.id).unwrap().files["ca.crt"], b"CA");

        store.remove(&a.id).unwrap();
        assert_eq!(store.connections().len(), 1);
        assert!(matches!(store.find("office"), Err(Error::NotFound(_))));
    }

    #[test]
    fn folders_and_order() {
        let (tmp, store) = store();
        let a = store.import(&ovpn(tmp.path(), "a", None)).unwrap();
        let b = store.import(&ovpn(tmp.path(), "b", None)).unwrap();
        store
            .set_tree(&[
                Node::Folder(Folder {
                    id: "f1".into(),
                    name: "Work".into(),
                    children: vec![Node::Connection(b.id.clone()), Node::Connection("gone".into())],
                    shared_credentials: true,
                    before_connect_script: None,
                    connected_script: None,
                    disconnected_script: None,
                }),
                Node::Connection(a.id.clone()),
            ])
            .unwrap();
        let conns = store.connections();
        assert_eq!(conns[0].id, b.id);
        assert_eq!(conns[0].folder.as_deref(), Some("f1"));
        assert_eq!(conns[1].id, a.id);
        assert!(store.folder("f1").unwrap().shared_credentials);
        let Node::Folder(f) = &store.tree()[0] else { panic!() };
        assert_eq!(f.children.len(), 1);
    }

    #[test]
    fn duplicate_rename_export_replace() {
        let (tmp, store) = store();
        let a = store.import(&ovpn(tmp.path(), "a", Some("Lab"))).unwrap();
        let copy = store.duplicate(&a.id).unwrap();
        assert_eq!(copy.name, "Lab copy");
        store.rename(&copy.id, "Lab 2").unwrap();
        assert_eq!(store.find(&copy.id).unwrap().name, "Lab 2");

        let out = tmp.path().join("Lab.fvpnz");
        store.export(&a.id, &out).unwrap();
        let replaced = store.import_replacing(&out, true).unwrap();
        assert_eq!(replaced.id, a.id);
        assert_eq!(store.connections().len(), 2);
    }
}
