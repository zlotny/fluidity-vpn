use std::path::PathBuf;

use fluidity_config::Bundle;
use fluidity_config::editor::EditorModel;
use fluidity_config::settings::Settings;

use crate::Store;

pub fn save_model(store: &Store, id: Option<&str>, model: &EditorModel) -> Result<String, String> {
    let problems = model.validate();
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    let (document, files) = model.to_bundle().map_err(|e| e.to_string())?;
    let mut bundle = Bundle { document, files };
    embed_scripts(&mut bundle)?;
    if let Some(id) = id {
        let mut settings = Settings::from_document(&bundle.document);
        let clash = store
            .connections()
            .into_iter()
            .any(|c| c.id != id && c.name == settings.name);
        if clash {
            settings.name = store.unique_name(&settings.name);
            settings.apply(&mut bundle.document);
        }
        if let Ok(existing) = store.load(id) {
            for (name, data) in existing.files {
                bundle.files.entry(name).or_insert(data);
            }
            let referenced: Vec<String> = fluidity_config::file_refs(&bundle.document)
                .into_iter()
                .map(|r| r.path)
                .chain(
                    fluidity_config::SCRIPT_META_KEYS
                        .iter()
                        .filter_map(|k| bundle.document.meta(k).map(str::to_string)),
                )
                .collect();
            bundle.files.retain(|name, _| referenced.contains(name));
        }
        store.save(id, &bundle).map_err(|e| e.to_string())?;
        return Ok(id.to_string());
    }
    let name = model.name.clone();
    store.add(bundle, &name).map(|c| c.id).map_err(|e| e.to_string())
}

fn embed_scripts(bundle: &mut Bundle) -> Result<(), String> {
    for key in fluidity_config::SCRIPT_META_KEYS {
        let Some(path) = bundle.document.meta(key).map(PathBuf::from) else {
            continue;
        };
        if !path.is_absolute() {
            continue;
        }
        let data = std::fs::read(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| key.to_string());
        let name = if bundle.files.get(&name).is_some_and(|d| *d != data) {
            format!("{key}-{name}")
        } else {
            name
        };
        bundle.document.set_meta(key, &[&name]);
        bundle.files.insert(name, data);
    }
    Ok(())
}
