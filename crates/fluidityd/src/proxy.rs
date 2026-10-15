use std::collections::HashMap;

use fluidity_mgmt::ProxyAction;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SystemProxies {
    pub https: Option<(String, u16)>,
    pub http: Option<(String, u16)>,
    pub socks: Option<(String, u16)>,
    pub exceptions: Vec<String>,
    pub exclude_simple: bool,
}

impl SystemProxies {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn parse_scutil(text: &str) -> Self {
        let mut values: HashMap<String, String> = HashMap::new();
        let mut exceptions = Vec::new();
        let mut in_exceptions = false;
        for line in text.lines().map(str::trim) {
            if in_exceptions {
                if line == "}" {
                    in_exceptions = false;
                } else if let Some((_, value)) = line.split_once(" : ") {
                    exceptions.push(value.trim().to_string());
                }
                continue;
            }
            if let Some((key, value)) = line.split_once(" : ") {
                if key == "ExceptionsList" {
                    in_exceptions = true;
                } else {
                    values.insert(key.to_string(), value.trim().to_string());
                }
            }
        }
        let server = |prefix: &str| {
            if values.get(&format!("{prefix}Enable")).map(String::as_str) != Some("1") {
                return None;
            }
            let host = values.get(&format!("{prefix}Proxy"))?.clone();
            let port = values.get(&format!("{prefix}Port"))?.parse().ok()?;
            (!host.is_empty()).then_some((host, port))
        };
        Self {
            https: server("HTTPS"),
            http: server("HTTP"),
            socks: server("SOCKS"),
            exceptions,
            exclude_simple: values.get("ExcludeSimpleHostnames").map(String::as_str) == Some("1"),
        }
    }

    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        if let Ok(text) = crate::net::run("/usr/sbin/scutil", &["--proxy"]) {
            return Self::parse_scutil(&text);
        }
        Self::default()
    }

    fn bypassed(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if self.exclude_simple && !host.contains('.') && !host.contains(':') {
            return true;
        }
        self.exceptions.iter().any(|rule| {
            let rule = rule.trim().to_ascii_lowercase();
            if let Some(suffix) = rule.strip_prefix("*.") {
                return host == suffix || host.ends_with(&format!(".{suffix}"));
            }
            if let Some((net, bits)) = rule.split_once('/') {
                let octets = bits.parse::<usize>().map(|b| b / 8).unwrap_or(4);
                let prefix: Vec<&str> = net.split('.').take(octets).collect();
                let parts: Vec<&str> = host.split('.').collect();
                return parts.len() == 4 && parts.iter().all(|p| p.parse::<u8>().is_ok()) && parts.starts_with(&prefix);
            }
            host == rule
        })
    }

    pub fn choose(&self, host: &str, proto: &str) -> ProxyAction {
        if !proto.to_ascii_uppercase().starts_with("TCP") || self.bypassed(host) {
            return ProxyAction::None;
        }
        if let Some((host, port)) = self.https.clone().or_else(|| self.http.clone()) {
            return ProxyAction::Http { host, port, nct: false };
        }
        match self.socks.clone() {
            Some((host, port)) => ProxyAction::Socks { host, port },
            None => ProxyAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : 169.254/16
    2 : intranet.example
  }
  ExcludeSimpleHostnames : 1
  FTPPassive : 1
  HTTPEnable : 1
  HTTPPort : 3128
  HTTPProxy : proxy.example
  SOCKSEnable : 0
  SOCKSPort : 1080
  SOCKSProxy : socks.example
}
";

    #[test]
    fn parses_scutil_output() {
        let p = SystemProxies::parse_scutil(SAMPLE);
        assert_eq!(p.http, Some(("proxy.example".into(), 3128)));
        assert_eq!(p.https, None);
        assert_eq!(p.socks, None);
        assert_eq!(p.exceptions, ["*.local", "169.254/16", "intranet.example"]);
        assert!(p.exclude_simple);
    }

    #[test]
    fn chooses_proxy_per_host() {
        let p = SystemProxies::parse_scutil(SAMPLE);
        let via = ProxyAction::Http {
            host: "proxy.example".into(),
            port: 3128,
            nct: false,
        };
        assert_eq!(p.choose("vpn.example.com", "TCP"), via);
        assert_eq!(p.choose("vpn.example.com", "UDP"), ProxyAction::None);
        assert_eq!(p.choose("printer.local", "TCP"), ProxyAction::None);
        assert_eq!(p.choose("169.254.1.2", "TCP"), ProxyAction::None);
        assert_eq!(p.choose("intranet.example", "TCP"), ProxyAction::None);
        assert_eq!(p.choose("vpnhost", "TCP"), ProxyAction::None);
        assert_eq!(SystemProxies::default().choose("a.b", "TCP"), ProxyAction::None);
    }
}
