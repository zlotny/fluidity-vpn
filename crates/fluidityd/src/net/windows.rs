use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{Backend, run, state_file, state_files};
use crate::killswitch::{Rules, parse_profile_actions, windows_release_script, windows_script};
use crate::netconfig::{NetConfig, ResolvedDns};

const NRPT: &str = r"HKLM\SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsPolicyConfig";
const TCPIP: &str = r"HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters";
const BLOCKED_V6: &str = "2000::/3,fc00::/7";
const KILL_SWITCH_GROUP: &str = "Fluidity Kill Switch";

pub struct WindowsBackend {
    state_dir: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    nrpt_key: Option<String>,
    search_list: Option<String>,
    firewall_rule: Option<String>,
}

pub fn rule_name(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .take(40)
        .collect();
    format!("Fluidity-{safe}")
}

pub fn nrpt_namespaces(cfg: &NetConfig) -> Vec<String> {
    match cfg.dns {
        Some(ResolvedDns::Full) => vec![".".into()],
        Some(ResolvedDns::Split) => cfg
            .match_domains
            .iter()
            .map(|d| format!(".{}", d.trim_start_matches('.')))
            .collect(),
        None => Vec::new(),
    }
}

pub fn parse_reg_value(output: &str, name: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        if parts.next()? != name {
            return None;
        }
        parts.next()?;
        Some(parts.collect::<Vec<_>>().join(" "))
    })
}

pub fn merged_search_list(original: &str, domains: &[String]) -> String {
    let mut list: Vec<String> = domains.to_vec();
    for d in original.split(',').map(str::trim).filter(|d| !d.is_empty()) {
        if !list.iter().any(|x| x.eq_ignore_ascii_case(d)) {
            list.push(d.to_string());
        }
    }
    list.join(",")
}

fn powershell(script: &str) -> Result<String, String> {
    run(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ],
    )
}

fn reg_add(key: &str, name: &str, kind: &str, data: &str) -> Result<(), String> {
    run("reg", &["add", key, "/v", name, "/t", kind, "/d", data, "/f"]).map(|_| ())
}

fn search_list() -> String {
    run("reg", &["query", TCPIP, "/v", "SearchList"])
        .ok()
        .and_then(|out| parse_reg_value(&out, "SearchList"))
        .unwrap_or_default()
}

impl WindowsBackend {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            state_dir: state_dir.to_path_buf(),
        }
    }

    fn apply_dns(&self, saved: &mut Saved, id: &str, cfg: &NetConfig) -> Result<(), String> {
        if cfg.dns.is_none() {
            return Ok(());
        }
        let namespaces = nrpt_namespaces(cfg);
        if !namespaces.is_empty() {
            let key = format!(r"{NRPT}\{}", rule_name(id));
            saved.nrpt_key = Some(key.clone());
            reg_add(&key, "Name", "REG_MULTI_SZ", &namespaces.join(r"\0"))?;
            reg_add(&key, "GenericDNSServers", "REG_SZ", &cfg.servers.join(";"))?;
            reg_add(&key, "ConfigOptions", "REG_DWORD", "8")?;
            reg_add(&key, "Version", "REG_DWORD", "2")?;
        }
        if !cfg.search_domains.is_empty() {
            let original = search_list();
            saved.search_list = Some(original.clone());
            reg_add(
                TCPIP,
                "SearchList",
                "REG_SZ",
                &merged_search_list(&original, &cfg.search_domains),
            )?;
        }
        let _ = run("ipconfig", &["/flushdns"]);
        Ok(())
    }

    fn apply_ipv6_block(&self, saved: &mut Saved, id: &str) -> Result<(), String> {
        let name = rule_name(id);
        saved.firewall_rule = Some(name.clone());
        run(
            "netsh",
            &[
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &format!("name={name}"),
                "dir=out",
                "action=block",
                &format!("remoteip={BLOCKED_V6}"),
            ],
        )
        .map(|_| ())
    }

    fn release_kill_switch(&self) {
        let saved_path = self.state_dir.join("killswitch.json");
        let Ok(data) = fs::read(&saved_path) else { return };
        let actions: Vec<(String, String)> = serde_json::from_slice(&data).unwrap_or_default();
        if powershell(&windows_release_script(KILL_SWITCH_GROUP, &actions)).is_ok() {
            let _ = fs::remove_file(saved_path);
        }
    }

    fn persist(&self, id: &str, saved: &Saved) {
        let _ = fs::create_dir_all(&self.state_dir);
        if let Ok(json) = serde_json::to_vec(saved) {
            let _ = fs::write(state_file(&self.state_dir, id), json);
        }
    }

    fn undo(saved: &Saved) {
        if let Some(key) = &saved.nrpt_key {
            let _ = run("reg", &["delete", key, "/f"]);
        }
        if let Some(original) = &saved.search_list {
            let _ = reg_add(TCPIP, "SearchList", "REG_SZ", original);
        }
        if let Some(name) = &saved.firewall_rule {
            let _ = run(
                "netsh",
                &["advfirewall", "firewall", "delete", "rule", &format!("name={name}")],
            );
        }
        if saved.nrpt_key.is_some() || saved.search_list.is_some() {
            let _ = run("ipconfig", &["/flushdns"]);
        }
    }
}

impl Backend for WindowsBackend {
    fn apply(&self, id: &str, cfg: &NetConfig) -> Result<(), String> {
        self.revert(id);
        if cfg.is_empty() {
            return Ok(());
        }
        let mut saved = Saved::default();
        let result = self.apply_dns(&mut saved, id, cfg).and_then(|_| {
            if cfg.block_ipv6 {
                self.apply_ipv6_block(&mut saved, id)
            } else {
                Ok(())
            }
        });
        self.persist(id, &saved);
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

    fn kill_switch(&self, rules: Option<&Rules>) -> Result<(), String> {
        let saved_path = self.state_dir.join("killswitch.json");
        match rules {
            Some(rules) => {
                if !saved_path.exists() {
                    let output = powershell(
                        "Get-NetFirewallProfile | ForEach-Object { \"$($_.Name)=$($_.DefaultOutboundAction)\" }",
                    )?;
                    let actions = parse_profile_actions(&output);
                    let _ = fs::create_dir_all(&self.state_dir);
                    fs::write(&saved_path, serde_json::to_vec(&actions).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                }
                powershell(&windows_script(rules, KILL_SWITCH_GROUP)).map(|_| ())
            }
            None => {
                self.release_kill_switch();
                Ok(())
            }
        }
    }

    fn recover(&self) {
        self.release_kill_switch();
        for path in state_files(&self.state_dir) {
            if let Ok(data) = fs::read(&path)
                && let Ok(saved) = serde_json::from_slice::<Saved>(&data)
            {
                Self::undo(&saved);
            }
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_nrpt_namespaces() {
        let mut cfg = NetConfig {
            dns: Some(ResolvedDns::Split),
            match_domains: vec!["corp.example".into(), ".lab".into()],
            ..NetConfig::default()
        };
        assert_eq!(nrpt_namespaces(&cfg), vec![".corp.example", ".lab"]);
        cfg.dns = Some(ResolvedDns::Full);
        assert_eq!(nrpt_namespaces(&cfg), vec!["."]);
    }

    #[test]
    fn parses_registry_output() {
        let out = "\r\nHKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Services\\Tcpip\\Parameters\r\n    SearchList    REG_SZ    home.lan,office\r\n\r\n";
        assert_eq!(parse_reg_value(out, "SearchList").as_deref(), Some("home.lan,office"));
        assert_eq!(parse_reg_value(out, "Other"), None);
    }

    #[test]
    fn merges_search_list() {
        assert_eq!(
            merged_search_list("home.lan, corp", &["corp".into(), "vpn".into()]),
            "corp,vpn,home.lan"
        );
        assert_eq!(rule_name("a b/c"), "Fluidity-a_b_c");
    }
}
