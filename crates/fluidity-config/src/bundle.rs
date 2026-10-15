use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use crate::Error;
use crate::document::{Document, Item};
use crate::files::{file_refs, set_ref};

pub const CONFIG_FILE: &str = "config.conf";
pub const BUNDLE_EXTENSION: &str = "fvpn";
pub const ARCHIVE_EXTENSION: &str = "fvpnz";
pub const SCRIPT_META_KEYS: [&str; 3] = ["preconnectscript", "connectedscript", "disconnectedscript"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bundle {
    pub document: Document,
    pub files: BTreeMap<String, Vec<u8>>,
}

trait Source {
    fn read(&self, path: &str) -> Result<Vec<u8>, Error>;
}

struct DirSource(PathBuf);

impl Source for DirSource {
    fn read(&self, path: &str) -> Result<Vec<u8>, Error> {
        let full = self.0.join(path);
        fs::read(&full).map_err(|source| Error::Io { path: full, source })
    }
}

struct MemSource {
    root: String,
    files: BTreeMap<String, Vec<u8>>,
}

impl Source for MemSource {
    fn read(&self, path: &str) -> Result<Vec<u8>, Error> {
        let rel = normalize(path).ok_or_else(|| Error::UnsafePath(path.to_string()))?;
        let key = if self.root.is_empty() {
            rel
        } else {
            format!("{}/{rel}", self.root)
        };
        self.files
            .get(&key)
            .cloned()
            .ok_or(Error::MissingFile(path.to_string()))
    }
}

fn normalize(path: &str) -> Option<String> {
    let mut parts = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(p) => parts.push(p.to_str()?.to_string()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn base_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("file")
        .to_string()
}

fn unique_name(files: &BTreeMap<String, Vec<u8>>, wanted: &str, data: &[u8]) -> String {
    if files.get(wanted).is_none_or(|existing| existing == data) {
        return wanted.to_string();
    }
    let (stem, ext) = match wanted.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (wanted, String::new()),
    };
    (2..)
        .map(|n| format!("{stem}-{n}{ext}"))
        .find(|name| files.get(name).is_none_or(|existing| existing == data))
        .expect("unbounded range")
}

impl Bundle {
    fn collect(mut document: Document, source: &dyn Source) -> Result<Self, Error> {
        let mut files = BTreeMap::new();

        for file_ref in file_refs(&document) {
            let data = source.read(&file_ref.path)?;
            let name = unique_name(&files, &base_name(&file_ref.path), &data);
            set_ref(&mut document, &file_ref, &name);
            files.insert(name, data);
        }

        for index in 0..document.items.len() {
            let Item::Meta { key, args } = &document.items[index] else {
                continue;
            };
            if !SCRIPT_META_KEYS.contains(&key.as_str()) {
                continue;
            }
            let Some(path) = args.first().filter(|p| !p.is_empty()).cloned() else {
                continue;
            };
            let data = source.read(&path)?;
            let name = unique_name(&files, &base_name(&path), &data);
            if let Item::Meta { args, .. } = &mut document.items[index] {
                args[0] = name.clone();
            }
            files.insert(name, data);
        }

        Ok(Self { document, files })
    }

    pub fn from_config_file(path: &Path) -> Result<Self, Error> {
        let text = fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.into(),
            source,
        })?;
        let document = Document::parse(&text)?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        Self::collect(document, &DirSource(dir))
    }

    pub fn from_dir(dir: &Path) -> Result<Self, Error> {
        Self::from_config_file(&find_config_in_dir(dir)?)
    }

    pub fn from_archive(data: &[u8]) -> Result<Self, Error> {
        let entries = if data.starts_with(&[0x1f, 0x8b]) {
            read_tar_gz(data)?
        } else if data.starts_with(b"PK") {
            read_zip(data)?
        } else {
            return Err(Error::UnknownFormat);
        };
        let config = entries
            .keys()
            .filter(|k| is_config_name(k))
            .min_by_key(|k| (!k.ends_with(CONFIG_FILE), k.matches('/').count()))
            .cloned()
            .ok_or(Error::NoConfig)?;
        let root = config.rsplit_once('/').map(|(r, _)| r.to_string()).unwrap_or_default();
        let text = String::from_utf8(entries[&config].clone()).map_err(|_| Error::NotUtf8)?;
        let document = Document::parse(&text)?;
        Self::collect(document, &MemSource { root, files: entries })
    }

    pub fn import(path: &Path) -> Result<Self, Error> {
        if path.is_dir() {
            return Self::from_dir(path);
        }
        let data = fs::read(path).map_err(|source| Error::Io {
            path: path.into(),
            source,
        })?;
        if is_archive(&data) {
            Self::from_archive(&data)
        } else {
            Self::from_config_file(path)
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.document.meta("name")
    }

    pub fn write_dir(&self, dir: &Path) -> Result<(), Error> {
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| Error::Io { path, source }
        };
        fs::create_dir_all(dir).map_err(io(dir))?;
        let config = dir.join(CONFIG_FILE);
        fs::write(&config, self.document.to_string()).map_err(io(&config))?;
        for (name, data) in &self.files {
            let name = normalize(name)
                .filter(|n| !n.contains('/'))
                .ok_or_else(|| Error::UnsafePath(name.clone()))?;
            let path = dir.join(name);
            fs::write(&path, data).map_err(io(&path))?;
        }
        Ok(())
    }

    pub fn to_archive(&self, bundle_name: &str) -> Result<Vec<u8>, Error> {
        let root = format!("{}.{BUNDLE_EXTENSION}", base_name(bundle_name));
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        let mut append = |name: &str, data: &[u8]| -> std::io::Result<()> {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            builder.append_data(&mut header, format!("{root}/{name}"), data)
        };
        let wrap = |source| Error::Io {
            path: PathBuf::from(&root),
            source,
        };
        append(CONFIG_FILE, self.document.to_string().as_bytes()).map_err(wrap)?;
        for (name, data) in &self.files {
            append(name, data).map_err(wrap)?;
        }
        builder.into_inner().and_then(|gz| gz.finish()).map_err(wrap)
    }
}

fn is_archive(data: &[u8]) -> bool {
    data.starts_with(&[0x1f, 0x8b]) || data.starts_with(b"PK\x03\x04")
}

fn is_config_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    !base.starts_with("._") && (base.ends_with(".conf") || base.ends_with(".ovpn"))
}

fn find_config_in_dir(dir: &Path) -> Result<PathBuf, Error> {
    let candidates = [dir.to_path_buf(), dir.join("Contents").join("Resources")];
    for candidate in &candidates {
        let preferred = candidate.join(CONFIG_FILE);
        if preferred.is_file() {
            return Ok(preferred);
        }
        let Ok(entries) = fs::read_dir(candidate) else { continue };
        let mut found: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.to_str().is_some_and(is_config_name))
            .collect();
        found.sort();
        if let Some(first) = found.into_iter().next() {
            return Ok(first);
        }
    }
    Err(Error::NoConfig)
}

fn read_tar_gz(data: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    let mut archive = tar::Archive::new(GzDecoder::new(data));
    let mut out = BTreeMap::new();
    let bad = |_| Error::UnknownFormat;
    for entry in archive.entries().map_err(bad)? {
        let mut entry = entry.map_err(bad)?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(bad)?.to_string_lossy().into_owned();
        let Some(name) = normalize(&path) else { continue };
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(bad)?;
        out.insert(name, buf);
    }
    Ok(out)
}

fn read_zip(data: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    let mut archive = zip::ZipArchive::new(Cursor::new(data)).map_err(|_| Error::UnknownFormat)?;
    let mut out = BTreeMap::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|_| Error::UnknownFormat)?;
        if !file.is_file() {
            continue;
        }
        let Some(name) = file.enclosed_name().and_then(|p| p.to_str().and_then(normalize)) else {
            continue;
        };
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).map_err(|_| Error::UnknownFormat)?;
        out.insert(name, buf);
    }
    Ok(out)
}
