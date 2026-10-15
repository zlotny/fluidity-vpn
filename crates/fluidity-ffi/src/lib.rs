use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;

use fluidity_config::Bundle;
use fluidity_config::editor::EditorModel;
use fluidity_config::settings::Settings;
use fluidity_core::{CredentialStore, Credentials, Node, Preferences, Store, wire_bundle};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Id {
    id: String,
}

fn store() -> Result<Store, String> {
    Store::open_default().map_err(|e| e.to_string())
}

fn parse<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, String> {
    serde_json::from_value(params.clone()).map_err(|e| format!("invalid parameters: {e}"))
}

fn connection_json(store: &Store, c: &fluidity_core::Connection) -> Value {
    let s = &c.settings;
    let folder = c.folder.as_deref().and_then(|f| store.folder(f));
    let scope = folder
        .as_ref()
        .filter(|f| f.shared_credentials)
        .map(|f| f.id.clone())
        .unwrap_or_else(|| c.id.clone());
    let script = |own: &Option<String>, shared: Option<&Option<String>>| {
        let own = own.as_ref().map(|name| c.dir.join(name).to_string_lossy().into_owned());
        own.or_else(|| shared.cloned().flatten())
    };
    json!({
        "id": c.id,
        "name": c.name,
        "folder": c.folder,
        "credential_scope": scope,
        "auto_reconnect": s.auto_reconnect,
        "connect_on_launch": s.connect_on_launch,
        "before_connect_script": script(&s.scripts.before_connect, folder.as_ref().map(|f| &f.before_connect_script)),
        "connected_script": script(&s.scripts.connected, folder.as_ref().map(|f| &f.connected_script)),
        "disconnected_script": script(&s.scripts.disconnected, folder.as_ref().map(|f| &f.disconnected_script)),
        "static_challenge_password": s.static_challenge_password.as_ref().map(|p| json!({"text": p.text, "delimiter": p.delimiter})),
        "static_challenge_acknowledge": s.static_challenge_acknowledge,
        "system_identity": s.system_identity,
        "openvpn_version": s.openvpn_version,
    })
}

fn save_model(store: &Store, id: Option<&str>, model: &EditorModel) -> Result<String, String> {
    let problems = model.validate();
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    let (document, files) = model.to_bundle().map_err(|e| e.to_string())?;
    let mut bundle = Bundle { document, files };
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

fn dispatch(method: &str, params: &Value) -> Result<Value, String> {
    match method {
        "version" => Ok(json!(env!("CARGO_PKG_VERSION"))),
        "data_dir" => Ok(json!(fluidity_core::default_data_dir())),
        "store.tree" => Ok(serde_json::to_value(store()?.tree()).unwrap_or_default()),
        "store.set_tree" => {
            #[derive(Deserialize)]
            struct P {
                tree: Vec<Node>,
            }
            let p: P = parse(params)?;
            store()?.set_tree(&p.tree).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "store.connections" => {
            let store = store()?;
            Ok(Value::Array(
                store.connections().iter().map(|c| connection_json(&store, c)).collect(),
            ))
        }
        "store.connection" => {
            let p: Id = parse(params)?;
            let store = store()?;
            let c = store.find(&p.id).map_err(|e| e.to_string())?;
            Ok(connection_json(&store, &c))
        }
        "store.import" => {
            #[derive(Deserialize)]
            struct P {
                path: PathBuf,
                #[serde(default)]
                replace: bool,
            }
            let p: P = parse(params)?;
            let store = store()?;
            let c = store.import_replacing(&p.path, p.replace).map_err(|e| e.to_string())?;
            Ok(connection_json(&store, &c))
        }
        "store.export" => {
            #[derive(Deserialize)]
            struct P {
                id: String,
                path: PathBuf,
            }
            let p: P = parse(params)?;
            store()?.export(&p.id, &p.path).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "store.remove" => {
            let p: Id = parse(params)?;
            store()?.remove(&p.id).map_err(|e| e.to_string())?;
            CredentialStore::clear(&p.id);
            Ok(Value::Null)
        }
        "store.duplicate" => {
            let p: Id = parse(params)?;
            let store = store()?;
            let c = store.duplicate(&p.id).map_err(|e| e.to_string())?;
            Ok(connection_json(&store, &c))
        }
        "store.rename" => {
            #[derive(Deserialize)]
            struct P {
                id: String,
                name: String,
            }
            let p: P = parse(params)?;
            store()?.rename(&p.id, &p.name).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "store.bundle" => {
            let p: Id = parse(params)?;
            let bundle = store()?.load(&p.id).map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(wire_bundle(&bundle)).unwrap_or_default())
        }
        "editor.new" => Ok(serde_json::to_value(EditorModel::new_connection()).unwrap_or_default()),
        "editor.load" => {
            let p: Id = parse(params)?;
            let bundle = store()?.load(&p.id).map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(EditorModel::from_document(&bundle.document, &bundle.files)).unwrap_or_default())
        }
        "editor.validate" => {
            let model: EditorModel = parse(params)?;
            Ok(json!(model.validate()))
        }
        "editor.save" => {
            #[derive(Deserialize)]
            struct P {
                id: Option<String>,
                model: EditorModel,
            }
            let p: P = parse(params)?;
            let id = save_model(&store()?, p.id.as_deref(), &p.model)?;
            Ok(json!(id))
        }
        "preferences.load" => {
            let store = store()?;
            Ok(serde_json::to_value(Preferences::load(store.root())).unwrap_or_default())
        }
        "preferences.save" => {
            let prefs: Preferences = parse(params)?;
            prefs.save(store()?.root()).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "credentials.load" => {
            #[derive(Deserialize)]
            struct P {
                scope: String,
            }
            let p: P = parse(params)?;
            let c = CredentialStore::load(&p.scope);
            Ok(json!({"username": c.username, "password": c.password}))
        }
        "credentials.save" => {
            #[derive(Deserialize)]
            struct P {
                scope: String,
                username: Option<String>,
                password: Option<String>,
            }
            let p: P = parse(params)?;
            CredentialStore::save(
                &p.scope,
                &Credentials {
                    username: p.username,
                    password: p.password,
                },
            )?;
            Ok(Value::Null)
        }
        "credentials.clear" => {
            #[derive(Deserialize)]
            struct P {
                scope: Option<String>,
            }
            let p: P = parse(params)?;
            match p.scope {
                Some(scope) => CredentialStore::clear(&scope),
                None => {
                    let store = store()?;
                    for c in store.connections() {
                        CredentialStore::clear(&c.id);
                        if let Some(f) = &c.folder {
                            CredentialStore::clear(f);
                        }
                    }
                }
            }
            Ok(Value::Null)
        }
        _ => Err(format!("unknown method {method}")),
    }
}

pub fn call(method: &str, params: &str) -> String {
    let params: Value = if params.trim().is_empty() {
        Value::Null
    } else {
        match serde_json::from_str(params) {
            Ok(v) => v,
            Err(e) => return json!({"error": format!("invalid JSON: {e}")}).to_string(),
        }
    };
    match dispatch(method, &params) {
        Ok(ok) => json!({ "ok": ok }).to_string(),
        Err(error) => json!({ "error": error }).to_string(),
    }
}

/// # Safety
/// `method` and `params` must be valid NUL-terminated strings or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fluidity_call(method: *const c_char, params: *const c_char) -> *mut c_char {
    let read = |p: *const c_char| {
        if p.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
        }
    };
    let result = std::panic::catch_unwind(|| call(&read(method), &read(params)))
        .unwrap_or_else(|_| json!({"error": "internal error"}).to_string());
    CString::new(result).unwrap_or_default().into_raw()
}

/// # Safety
/// `value` must come from `fluidity_call` and be freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fluidity_free(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_flow_through_store() {
        let tmp = std::env::temp_dir().join(format!("fluidity-ffi-{}", std::process::id()));
        unsafe { std::env::set_var("FLUIDITY_HOME", &tmp) };
        let new: Value = serde_json::from_str(&call("editor.new", "")).unwrap();
        let mut model = new["ok"].clone();
        model["name"] = json!("Lab");
        model["remotes"][0]["host"] = json!("vpn.example.com");
        model["ca"] = json!({"name": "ca.crt", "data": "Q0E="});
        let saved: Value =
            serde_json::from_str(&call("editor.save", &json!({"id": null, "model": model}).to_string())).unwrap();
        let id = saved["ok"].as_str().unwrap().to_string();
        let list: Value = serde_json::from_str(&call("store.connections", "")).unwrap();
        assert_eq!(list["ok"][0]["name"], "Lab");
        let bundle: Value = serde_json::from_str(&call("store.bundle", &json!({"id": id}).to_string())).unwrap();
        assert!(
            bundle["ok"]["config"]
                .as_str()
                .unwrap()
                .contains("remote vpn.example.com 1194 udp")
        );
        assert_eq!(bundle["ok"]["files"]["ca.crt"], "Q0E=");
        let bad: Value = serde_json::from_str(&call("nope", "")).unwrap();
        assert!(bad["error"].is_string());
        let _ = std::fs::remove_dir_all(tmp);
    }
}
