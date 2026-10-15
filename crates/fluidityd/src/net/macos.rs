use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::propertylist::{CFPropertyList, CFPropertyListSubClass};
use core_foundation::string::CFString;
use serde::{Deserialize, Serialize};
use system_configuration::dynamic_store::{SCDynamicStore, SCDynamicStoreBuilder};

use super::{Backend, run, run_with_input, state_file, state_files};
use crate::netconfig::{NetConfig, ResolvedDns};

pub struct MacBackend {
    state_dir: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    keys_created: Vec<String>,
    restore: Vec<(String, Option<String>)>,
    pf_anchor: Option<String>,
    pf_token: Option<String>,
}

fn store() -> Result<SCDynamicStore, String> {
    SCDynamicStoreBuilder::new("fluidityd")
        .build()
        .ok_or_else(|| "cannot open the system configuration store".to_string())
}

fn encode(value: &CFPropertyList) -> Option<String> {
    let data = core_foundation::propertylist::create_data(
        value.as_concrete_TypeRef(),
        core_foundation::propertylist::kCFPropertyListXMLFormat_v1_0,
    )
    .ok()?;
    Some(STANDARD.encode(data.bytes()))
}

fn decode(text: &str) -> Option<CFPropertyList> {
    let data = STANDARD.decode(text).ok()?;
    let (ptr, _) = core_foundation::propertylist::create_with_data(
        core_foundation::data::CFData::from_buffer(&data),
        core_foundation::propertylist::kCFPropertyListImmutable,
    )
    .ok()?;
    Some(unsafe { CFPropertyList::wrap_under_create_rule(ptr) })
}

fn strings(values: &[String]) -> CFType {
    let items: Vec<CFString> = values.iter().map(|v| CFString::new(v)).collect();
    CFArray::from_CFTypes(&items).as_CFType()
}

fn dict(pairs: Vec<(&str, CFType)>) -> CFDictionary {
    let pairs: Vec<(CFType, CFType)> = pairs
        .into_iter()
        .map(|(k, v)| (CFString::new(k).as_CFType(), v))
        .collect();
    CFDictionary::from_CFType_pairs(&pairs).to_untyped()
}

fn primary_service(store: &SCDynamicStore) -> Option<String> {
    let global = store.get("State:/Network/Global/IPv4")?;
    let global = global.downcast_into::<CFDictionary>()?;
    let value = global.find(CFString::from_static_string("PrimaryService").as_CFTypeRef() as *const _)?;
    let value = unsafe { CFType::wrap_under_get_rule(*value as _) };
    value.downcast::<CFString>().map(|s| s.to_string())
}

fn safe_id(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect()
}

impl MacBackend {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            state_dir: state_dir.to_path_buf(),
        }
    }

    fn time_machine_marker(&self) -> PathBuf {
        self.state_dir.join("timemachine-paused")
    }

    fn save_original(store: &SCDynamicStore, saved: &mut Saved, key: &str) {
        if saved.restore.iter().any(|(k, _)| k == key) {
            return;
        }
        let original = store.get(key).and_then(|v| encode(&v));
        saved.restore.push((key.to_string(), original));
    }

    fn set<V: CFPropertyListSubClass>(store: &SCDynamicStore, key: &str, value: V) -> Result<(), String> {
        if store.set(key, value) {
            Ok(())
        } else {
            Err(format!("cannot write {key} to the system configuration"))
        }
    }

    fn apply_dns(&self, store: &SCDynamicStore, saved: &mut Saved, id: &str, cfg: &NetConfig) -> Result<(), String> {
        let Some(mode) = cfg.dns else { return Ok(()) };
        let service = format!("State:/Network/Service/fluidity-{}/DNS", safe_id(id));
        let mut entries = vec![
            ("ServerAddresses", strings(&cfg.servers)),
            ("InterfaceName", CFString::new(&cfg.interface).as_CFType()),
        ];
        if !cfg.match_domains.is_empty() {
            entries.push(("SupplementalMatchDomains", strings(&cfg.match_domains)));
            entries.push(("SupplementalMatchDomainsNoSearch", CFNumber::from(0).as_CFType()));
        }
        if !cfg.search_domains.is_empty() {
            entries.push(("SearchDomains", strings(&cfg.search_domains)));
            entries.push(("DomainName", CFString::new(&cfg.search_domains[0]).as_CFType()));
        }
        Self::set(store, &service, dict(entries))?;
        saved.keys_created.push(service);

        if mode == ResolvedDns::Full {
            let primary = primary_service(store).ok_or("no primary network service")?;
            let mut full = vec![("ServerAddresses", strings(&cfg.servers))];
            if !cfg.search_domains.is_empty() {
                full.push(("SearchDomains", strings(&cfg.search_domains)));
                full.push(("DomainName", CFString::new(&cfg.search_domains[0]).as_CFType()));
            }
            for prefix in ["State", "Setup"] {
                let key = format!("{prefix}:/Network/Service/{primary}/DNS");
                if prefix == "Setup" && store.get(key.as_str()).is_none() {
                    continue;
                }
                Self::save_original(store, saved, &key);
                Self::set(store, &key, dict(full.clone()))?;
            }
        }

        if !cfg.wins.is_empty() {
            let key = format!("State:/Network/Service/fluidity-{}/SMB", safe_id(id));
            Self::set(store, &key, dict(vec![("WINSAddresses", strings(&cfg.wins))]))?;
            saved.keys_created.push(key);
        }
        Ok(())
    }

    fn apply_proxies(&self, store: &SCDynamicStore, saved: &mut Saved, cfg: &NetConfig) -> Result<(), String> {
        let mut entries: Vec<(&str, CFType)> = Vec::new();
        let on = || CFNumber::from(1).as_CFType();
        if let Some(url) = &cfg.proxy_auto {
            entries.push(("ProxyAutoConfigEnable", on()));
            entries.push(("ProxyAutoConfigURLString", CFString::new(url).as_CFType()));
        }
        for (enable, host_key, port_key, value) in [
            ("HTTPEnable", "HTTPProxy", "HTTPPort", &cfg.proxy_http),
            ("HTTPSEnable", "HTTPSProxy", "HTTPSPort", &cfg.proxy_https),
            ("SOCKSEnable", "SOCKSProxy", "SOCKSPort", &cfg.proxy_socks),
        ] {
            if let Some((host, port)) = value {
                entries.push((enable, on()));
                entries.push((host_key, CFString::new(host).as_CFType()));
                entries.push((port_key, CFNumber::from(*port as i32).as_CFType()));
            }
        }
        if entries.is_empty() {
            return Ok(());
        }
        entries.push(("ExceptionsList", strings(&["*.local".into(), "169.254/16".into()])));
        entries.push(("ExcludeSimpleHostnames", CFBoolean::true_value().as_CFType()));
        let key = "State:/Network/Global/Proxies";
        Self::save_original(store, saved, key);
        Self::set(store, key, dict(entries))
    }

    fn apply_ipv6_block(&self, saved: &mut Saved, id: &str) -> Result<(), String> {
        let anchor = format!("com.apple/fluidity-{}", safe_id(id));
        let rules = "pass out quick on lo0 inet6 all\n\
                     pass out quick inet6 to fe80::/10\n\
                     pass out quick inet6 to ff00::/8\n\
                     block drop out quick inet6 all\n\
                     block drop in quick inet6 from !fe80::/10 to any\n";
        run_with_input("/sbin/pfctl", &["-a", &anchor, "-f", "-"], rules)?;
        saved.pf_anchor = Some(anchor);
        let output = std::process::Command::new("/sbin/pfctl")
            .arg("-E")
            .output()
            .map_err(|e| format!("pfctl: {e}"))?;
        let text = String::from_utf8_lossy(&output.stderr);
        saved.pf_token = text
            .lines()
            .find_map(|l| l.strip_prefix("Token : "))
            .map(|t| t.trim().to_string());
        Ok(())
    }

    fn persist(&self, id: &str, saved: &Saved) {
        let _ = fs::create_dir_all(&self.state_dir);
        if let Ok(json) = serde_json::to_vec(saved) {
            let _ = fs::write(state_file(&self.state_dir, id), json);
        }
    }

    fn undo(saved: &Saved) {
        let Ok(store) = store() else { return };
        for key in &saved.keys_created {
            store.remove(key.as_str());
        }
        for (key, original) in saved.restore.iter().rev() {
            match original.as_deref().and_then(decode) {
                Some(value) => {
                    if let Some(d) = value.downcast_into::<CFDictionary>() {
                        store.set(key.as_str(), d);
                    }
                }
                None => {
                    store.remove(key.as_str());
                }
            }
        }
        if let Some(anchor) = &saved.pf_anchor {
            let _ = run("/sbin/pfctl", &["-a", anchor, "-F", "all"]);
        }
        if let Some(token) = &saved.pf_token {
            let _ = run("/sbin/pfctl", &["-X", token]);
        }
        let _ = run("/usr/bin/dscacheutil", &["-flushcache"]);
        let _ = run("/usr/bin/killall", &["-HUP", "mDNSResponder"]);
    }
}

impl Backend for MacBackend {
    fn apply(&self, id: &str, cfg: &NetConfig) -> Result<(), String> {
        self.revert(id);
        if cfg.is_empty() {
            return Ok(());
        }
        let store = store()?;
        let mut saved = Saved::default();
        let result = self
            .apply_dns(&store, &mut saved, id, cfg)
            .and_then(|_| self.apply_proxies(&store, &mut saved, cfg))
            .and_then(|_| {
                if cfg.block_ipv6 {
                    self.apply_ipv6_block(&mut saved, id)
                } else {
                    Ok(())
                }
            });
        self.persist(id, &saved);
        let _ = run("/usr/bin/dscacheutil", &["-flushcache"]);
        let _ = run("/usr/bin/killall", &["-HUP", "mDNSResponder"]);
        result
    }

    fn revert(&self, id: &str) {
        let path = state_file(&self.state_dir, id);
        let Ok(data) = fs::read(&path) else { return };
        if let Ok(saved) = serde_json::from_slice::<Saved>(&data) {
            Self::undo(&saved);
        }
        let _ = fs::remove_file(path);
    }

    fn recover(&self) {
        if let Err(err) = self.resume_time_machine() {
            tracing::warn!("Time Machine: {err}");
        }
        for path in state_files(&self.state_dir) {
            if let Ok(data) = fs::read(&path)
                && let Ok(saved) = serde_json::from_slice::<Saved>(&data)
            {
                Self::undo(&saved);
            }
            let _ = fs::remove_file(path);
        }
    }

    fn pause_time_machine(&self) -> Result<(), String> {
        let auto = run(
            "/usr/bin/defaults",
            &["read", "/Library/Preferences/com.apple.TimeMachine", "AutoBackup"],
        )
        .unwrap_or_default();
        if auto.trim() != "1" {
            return Ok(());
        }
        run("/usr/bin/tmutil", &["disable"])?;
        fs::create_dir_all(&self.state_dir).map_err(|e| e.to_string())?;
        fs::write(self.time_machine_marker(), b"").map_err(|e| e.to_string())
    }

    fn resume_time_machine(&self) -> Result<(), String> {
        let marker = self.time_machine_marker();
        if !marker.exists() {
            return Ok(());
        }
        run("/usr/bin/tmutil", &["enable"])?;
        fs::remove_file(marker).map_err(|e| e.to_string())
    }
}
