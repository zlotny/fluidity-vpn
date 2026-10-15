use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AddressDisplay {
    #[default]
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrafficDisplay {
    #[default]
    Rates,
    Totals,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphStyle {
    #[default]
    Line,
    Bars,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub first_run: bool,
    pub start_at_login: bool,
    pub disable_time_machine: bool,
    pub reconnect_on_wake: bool,
    pub reconnect_on_network: bool,
    pub idle_disconnect_minutes: Option<u32>,
    pub check_for_updates: bool,
    pub include_beta_updates: bool,
    pub notifications: bool,
    pub icon_set: String,
    pub address_display: AddressDisplay,
    pub traffic_display: TrafficDisplay,
    pub graph_style: GraphStyle,
    pub details_always_on_top: bool,
    pub reset_interfaces_on_disconnect: bool,
    pub block_ipv6: bool,
    pub kill_switch: bool,
    pub kill_switch_allow_lan: bool,
    pub openvpn_version: Option<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            first_run: true,
            start_at_login: false,
            disable_time_machine: false,
            reconnect_on_wake: true,
            reconnect_on_network: true,
            idle_disconnect_minutes: None,
            check_for_updates: true,
            include_beta_updates: false,
            notifications: true,
            icon_set: "default".into(),
            address_display: AddressDisplay::Ipv4,
            traffic_display: TrafficDisplay::Rates,
            graph_style: GraphStyle::Line,
            details_always_on_top: false,
            reset_interfaces_on_disconnect: false,
            block_ipv6: true,
            kill_switch: false,
            kill_switch_allow_lan: true,
            openvpn_version: None,
        }
    }
}

impl Preferences {
    pub fn file(root: &Path) -> std::path::PathBuf {
        root.join("preferences.json")
    }

    pub fn load(root: &Path) -> Self {
        let mut prefs: Self = fs::read(Self::file(root))
            .ok()
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default();
        if let Some(managed) = managed_overrides() {
            prefs.merge(&managed);
        }
        prefs
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        fs::create_dir_all(root)?;
        let path = Self::file(root);
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, path)
    }

    pub(crate) fn merge(&mut self, overrides: &serde_json::Value) {
        let Ok(mut current) = serde_json::to_value(&*self) else {
            return;
        };
        if let (Some(cur), Some(over)) = (current.as_object_mut(), overrides.as_object()) {
            for (k, v) in over {
                cur.insert(k.clone(), v.clone());
            }
        }
        if let Ok(merged) = serde_json::from_value(current) {
            *self = merged;
        }
    }
}

pub fn managed_file() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("FLUIDITY_MANAGED_PREFERENCES") {
        return p.into();
    }
    if cfg!(target_os = "macos") {
        "/Library/Application Support/Fluidity/managed.json".into()
    } else if cfg!(windows) {
        r"C:\ProgramData\Fluidity\managed.json".into()
    } else {
        "/etc/fluidity/managed.json".into()
    }
}

fn managed_overrides() -> Option<serde_json::Value> {
    serde_json::from_slice(&fs::read(managed_file()).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_partial_files() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            Preferences::file(tmp.path()),
            r#"{"block_ipv6": false, "graph_style": "bars"}"#,
        )
        .unwrap();
        let prefs = Preferences::load(tmp.path());
        assert!(!prefs.block_ipv6);
        assert_eq!(prefs.graph_style, GraphStyle::Bars);
        assert!(prefs.reconnect_on_wake);
        prefs.save(tmp.path()).unwrap();
        assert_eq!(Preferences::load(tmp.path()), prefs);
    }
}
