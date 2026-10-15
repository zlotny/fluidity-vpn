use std::collections::BTreeMap;
use std::net::{IpAddr, ToSocketAddrs};

use fluidity_config::Document;

pub const LAN_V4: &[&str] = &[
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "169.254.0.0/16",
    "224.0.0.0/4",
    "255.255.255.255/32",
];
pub const LAN_V6: &[&str] = &["fe80::/10", "fc00::/7", "ff00::/8"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    pub owner: u32,
    pub interface: String,
    pub servers: Vec<IpAddr>,
    pub allow_lan: bool,
    pub connected: bool,
    pub orphaned: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    pub interfaces: Vec<String>,
    pub servers: Vec<IpAddr>,
    pub allow_lan: bool,
    pub allow_dns: bool,
}

impl Rules {
    pub fn from_locks(locks: &BTreeMap<String, Lock>) -> Option<Self> {
        if locks.is_empty() {
            return None;
        }
        let mut rules = Rules {
            allow_lan: locks.values().all(|l| l.allow_lan),
            allow_dns: locks.values().any(|l| !l.connected),
            ..Rules::default()
        };
        for lock in locks.values() {
            if !lock.interface.is_empty() && !rules.interfaces.contains(&lock.interface) {
                rules.interfaces.push(lock.interface.clone());
            }
            for server in &lock.servers {
                if !rules.servers.contains(server) {
                    rules.servers.push(*server);
                }
            }
        }
        Some(rules)
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn v4(&self) -> Vec<String> {
        self.servers
            .iter()
            .filter(|a| a.is_ipv4())
            .map(IpAddr::to_string)
            .collect()
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn v6(&self) -> Vec<String> {
        self.servers
            .iter()
            .filter(|a| a.is_ipv6())
            .map(IpAddr::to_string)
            .collect()
    }
}

pub fn remote_servers(document: &Document, remote_ip: &str) -> Vec<IpAddr> {
    let mut out: Vec<IpAddr> = Vec::new();
    let mut push = |ip: IpAddr| {
        if !ip.is_loopback() && !out.contains(&ip) {
            out.push(ip);
        }
    };
    if let Ok(ip) = remote_ip.parse() {
        push(ip);
    }
    for args in document.directives("remote") {
        let Some(host) = args.first() else { continue };
        if let Ok(ip) = host.parse() {
            push(ip);
            continue;
        }
        let port = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(1194u16);
        if let Ok(addrs) = (host.as_str(), port).to_socket_addrs() {
            for addr in addrs {
                push(addr.ip());
            }
        }
    }
    out
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn pf_rules(rules: &Rules) -> String {
    let mut out = String::from("pass out quick on lo0 all\n");
    for iface in &rules.interfaces {
        out.push_str(&format!("pass out quick on {iface} all\n"));
    }
    if !rules.servers.is_empty() {
        let list: Vec<String> = rules.servers.iter().map(IpAddr::to_string).collect();
        out.push_str(&format!(
            "pass out quick proto {{ tcp udp }} to {{ {} }}\n",
            list.join(" ")
        ));
    }
    out.push_str("pass out quick inet proto udp from any port 68 to any port 67\n");
    out.push_str("pass out quick inet6 proto udp from any port 546 to any port 547\n");
    out.push_str("pass out quick inet6 proto ipv6-icmp all\n");
    if rules.allow_dns {
        out.push_str("pass out quick proto { tcp udp } to any port 53\n");
    }
    if rules.allow_lan {
        let lan: Vec<&str> = LAN_V4.iter().chain(LAN_V6).copied().collect();
        out.push_str(&format!("pass out quick to {{ {} }}\n", lan.join(" ")));
    }
    out.push_str("block drop out quick all\n");
    out
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn nft_rules(table: &str, rules: &Rules) -> String {
    let mut body = vec![
        "type filter hook output priority 0; policy accept;".to_string(),
        "oifname \"lo\" accept".to_string(),
    ];
    if !rules.interfaces.is_empty() {
        let names: Vec<String> = rules.interfaces.iter().map(|i| format!("\"{i}\"")).collect();
        body.push(format!("oifname {{ {} }} accept", names.join(", ")));
    }
    let v4 = rules.v4();
    if !v4.is_empty() {
        body.push(format!("ip daddr {{ {} }} accept", v4.join(", ")));
    }
    let v6 = rules.v6();
    if !v6.is_empty() {
        body.push(format!("ip6 daddr {{ {} }} accept", v6.join(", ")));
    }
    body.push("udp sport 68 udp dport 67 accept".into());
    body.push("udp sport 546 udp dport 547 accept".into());
    body.push("meta l4proto ipv6-icmp accept".into());
    if rules.allow_dns {
        body.push("meta l4proto { tcp, udp } th dport 53 accept".into());
    }
    if rules.allow_lan {
        body.push(format!("ip daddr {{ {} }} accept", LAN_V4.join(", ")));
        body.push(format!("ip6 daddr {{ {} }} accept", LAN_V6.join(", ")));
    }
    body.push("drop".into());
    format!(
        "table inet {table} {{\n  chain output {{\n    {}\n  }}\n}}\n",
        body.join("\n    ")
    )
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn windows_script(rules: &Rules, group: &str) -> String {
    let mut lines = vec![
        "$ErrorActionPreference = 'Stop'".to_string(),
        format!("Get-NetFirewallRule -Group '{group}' -ErrorAction SilentlyContinue | Remove-NetFirewallRule"),
    ];
    let mut allow = |name: &str, args: String| {
        lines.push(format!(
            "New-NetFirewallRule -Group '{group}' -DisplayName '{group} {name}' -Direction Outbound -Action Allow {args} | Out-Null"
        ));
    };
    for iface in &rules.interfaces {
        allow(
            &format!("tunnel {iface}"),
            format!("-InterfaceAlias '{}'", iface.replace('\'', "''")),
        );
    }
    if !rules.servers.is_empty() {
        let list: Vec<String> = rules.servers.iter().map(IpAddr::to_string).collect();
        allow("servers", format!("-RemoteAddress {}", list.join(",")));
    }
    allow("dhcp", "-Protocol UDP -LocalPort 68 -RemotePort 67".into());
    allow("dhcpv6", "-Protocol UDP -LocalPort 546 -RemotePort 547".into());
    allow("loopback", "-RemoteAddress 127.0.0.0/8,::1".into());
    if rules.allow_dns {
        allow("dns", "-Protocol UDP -RemotePort 53".into());
        allow("dns tcp", "-Protocol TCP -RemotePort 53".into());
    }
    if rules.allow_lan {
        let lan: Vec<&str> = LAN_V4.iter().chain(LAN_V6).copied().collect();
        allow("lan", format!("-RemoteAddress {}", lan.join(",")));
    }
    lines.push("Set-NetFirewallProfile -All -DefaultOutboundAction Block".into());
    lines.join("\n")
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn windows_release_script(group: &str, saved_actions: &[(String, String)]) -> String {
    let mut lines = vec![format!(
        "Get-NetFirewallRule -Group '{group}' -ErrorAction SilentlyContinue | Remove-NetFirewallRule"
    )];
    if saved_actions.is_empty() {
        lines.push("Set-NetFirewallProfile -All -DefaultOutboundAction Allow".into());
    }
    for (profile, action) in saved_actions {
        let action = if action.eq_ignore_ascii_case("Block") {
            "Block"
        } else {
            "Allow"
        };
        lines.push(format!(
            "Set-NetFirewallProfile -Name '{}' -DefaultOutboundAction {action}",
            profile.replace('\'', "''")
        ));
    }
    lines.join("\n")
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_profile_actions(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let (name, action) = line.trim().split_once('=')?;
            Some((name.trim().to_string(), action.trim().to_string()))
        })
        .filter(|(n, a)| !n.is_empty() && !a.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock(iface: &str, server: &str, lan: bool, connected: bool) -> Lock {
        Lock {
            owner: 501,
            interface: iface.into(),
            servers: vec![server.parse().unwrap()],
            allow_lan: lan,
            connected,
            orphaned: false,
        }
    }

    #[test]
    fn merges_locks() {
        assert_eq!(Rules::from_locks(&BTreeMap::new()), None);
        let mut locks = BTreeMap::new();
        locks.insert("a".to_string(), lock("utun4", "203.0.113.10", true, true));
        locks.insert("b".to_string(), lock("utun5", "2001:db8::1", false, true));
        let rules = Rules::from_locks(&locks).unwrap();
        assert_eq!(rules.interfaces, vec!["utun4", "utun5"]);
        assert!(!rules.allow_lan);
        assert!(!rules.allow_dns);
        locks.get_mut("b").unwrap().connected = false;
        assert!(Rules::from_locks(&locks).unwrap().allow_dns);
    }

    #[test]
    fn renders_pf() {
        let rules = Rules {
            interfaces: vec!["utun4".into()],
            servers: vec!["203.0.113.10".parse().unwrap(), "2001:db8::1".parse().unwrap()],
            allow_lan: true,
            allow_dns: false,
        };
        let pf = pf_rules(&rules);
        assert!(pf.contains("pass out quick on utun4 all\n"));
        assert!(pf.contains("to { 203.0.113.10 2001:db8::1 }"));
        assert!(pf.contains("192.168.0.0/16"));
        assert!(!pf.contains("port 53"));
        assert!(pf.ends_with("block drop out quick all\n"));
    }

    #[test]
    fn renders_nft() {
        let rules = Rules {
            interfaces: vec!["tun0".into()],
            servers: vec!["203.0.113.10".parse().unwrap()],
            allow_lan: false,
            allow_dns: true,
        };
        let nft = nft_rules("fluidity_killswitch", &rules);
        assert!(nft.starts_with("table inet fluidity_killswitch {"));
        assert!(nft.contains("oifname { \"tun0\" } accept"));
        assert!(nft.contains("ip daddr { 203.0.113.10 } accept"));
        assert!(!nft.contains("ip6 daddr"));
        assert!(nft.contains("th dport 53 accept"));
        assert!(nft.contains("    drop\n"));
    }

    #[test]
    fn renders_windows() {
        let rules = Rules {
            interfaces: vec!["OpenVPN Data Channel Offload".into()],
            servers: vec!["203.0.113.10".parse().unwrap()],
            allow_lan: true,
            allow_dns: false,
        };
        let ps = windows_script(&rules, "Fluidity Kill Switch");
        assert!(ps.contains("-InterfaceAlias 'OpenVPN Data Channel Offload'"));
        assert!(ps.contains("-RemoteAddress 203.0.113.10"));
        assert!(ps.ends_with("Set-NetFirewallProfile -All -DefaultOutboundAction Block"));
        let saved = parse_profile_actions("Domain=Allow\r\nPrivate=Block\r\nPublic=Allow\r\n");
        assert_eq!(saved.len(), 3);
        let release = windows_release_script("Fluidity Kill Switch", &saved);
        assert!(release.contains("-Name 'Private' -DefaultOutboundAction Block"));
        assert!(release.contains("-Name 'Public' -DefaultOutboundAction Allow"));
    }

    #[test]
    fn collects_remote_servers() {
        let doc = Document::parse("client\nremote 198.51.100.7 1194\nremote 127.0.0.1 1194\n").unwrap();
        let servers = remote_servers(&doc, "203.0.113.10");
        assert_eq!(
            servers,
            vec![
                "203.0.113.10".parse::<IpAddr>().unwrap(),
                "198.51.100.7".parse().unwrap()
            ]
        );
    }
}
