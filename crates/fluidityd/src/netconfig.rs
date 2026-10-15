use fluidity_config::Document;
use fluidity_config::settings::{DnsMode, HostPort, InterfaceProxies, Redirect, Settings};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolvedDns {
    Full,
    Split,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NetConfig {
    pub interface: String,
    pub dns: Option<ResolvedDns>,
    pub servers: Vec<String>,
    pub match_domains: Vec<String>,
    pub search_domains: Vec<String>,
    pub wins: Vec<String>,
    pub proxy_auto: Option<String>,
    pub proxy_http: Option<(String, u16)>,
    pub proxy_https: Option<(String, u16)>,
    pub proxy_socks: Option<(String, u16)>,
    pub block_ipv6: bool,
}

impl NetConfig {
    pub fn is_empty(&self) -> bool {
        self.dns.is_none()
            && self.proxy_auto.is_none()
            && self.proxy_http.is_none()
            && self.proxy_https.is_none()
            && self.proxy_socks.is_none()
            && !self.block_ipv6
    }
}

pub fn parse_push_reply(message: &str) -> Option<Vec<Vec<String>>> {
    let start = message.find("PUSH_REPLY,")? + "PUSH_REPLY,".len();
    let body = &message[start..];
    let body = body.strip_suffix('\'').unwrap_or(body);
    let body = body.split("' (status=").next().unwrap_or(body);
    Some(
        body.split(',')
            .filter_map(|opt| fluidity_config::tokenize(opt.trim(), 0).ok())
            .filter(|tokens| !tokens.is_empty())
            .collect(),
    )
}

#[derive(Debug, Default)]
struct DnsInfo {
    servers: Vec<String>,
    domains: Vec<String>,
    search: Vec<String>,
    wins: Vec<String>,
    proxy_auto: Option<String>,
    proxy_http: Option<(String, u16)>,
    proxy_https: Option<(String, u16)>,
    redirect: bool,
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    if !value.is_empty() && !list.iter().any(|v| v.eq_ignore_ascii_case(value)) {
        list.push(value.to_string());
    }
}

fn host_port(args: &[String]) -> Option<(String, u16)> {
    Some((args.first()?.clone(), args.get(1)?.parse().ok()?))
}

fn collect(options: &[Vec<String>], info: &mut DnsInfo, include_dns: bool) {
    for opt in options {
        let args = &opt[1..];
        match opt[0].as_str() {
            "redirect-gateway" => {
                if !args.iter().any(|a| a == "!ipv4") {
                    info.redirect = true;
                }
            }
            "dhcp-option" if include_dns => {
                let Some(kind) = args.first() else { continue };
                let rest = &args[1..];
                let value = rest.first().map(String::as_str).unwrap_or_default();
                match kind.to_ascii_uppercase().as_str() {
                    "DNS" | "DNS6" => push_unique(&mut info.servers, value),
                    "DOMAIN" | "ADAPTER_DOMAIN_SUFFIX" => push_unique(&mut info.domains, value),
                    "DOMAIN-SEARCH" => push_unique(&mut info.search, value),
                    "WINS" => push_unique(&mut info.wins, value),
                    "PROXY_AUTO_CONFIG_URL" => info.proxy_auto = Some(value.to_string()),
                    "PROXY_HTTP" => info.proxy_http = host_port(rest),
                    "PROXY_HTTPS" => info.proxy_https = host_port(rest),
                    _ => {}
                }
            }
            "dns" if include_dns => match args.first().map(String::as_str) {
                Some("server") => match args.get(2).map(String::as_str) {
                    Some("address") => {
                        for addr in &args[3..] {
                            let host = addr
                                .strip_prefix('[')
                                .and_then(|a| a.split_once(']').map(|(h, _)| h))
                                .unwrap_or_else(|| {
                                    if addr.matches(':').count() == 1 {
                                        addr.split(':').next().unwrap_or(addr)
                                    } else {
                                        addr
                                    }
                                });
                            push_unique(&mut info.servers, host);
                        }
                    }
                    Some("resolve-domains") => {
                        for d in &args[3..] {
                            push_unique(&mut info.domains, d);
                        }
                    }
                    _ => {}
                },
                Some("search-domains") => {
                    for d in &args[1..] {
                        push_unique(&mut info.search, d);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

pub fn document_options(doc: &Document) -> Vec<Vec<String>> {
    doc.items
        .iter()
        .filter_map(|item| match item {
            fluidity_config::Item::Directive { name, args } => {
                let mut v = vec![name.clone()];
                v.extend(args.iter().cloned());
                Some(v)
            }
            _ => None,
        })
        .collect()
}

pub struct Inputs<'a> {
    pub settings: &'a Settings,
    pub config: &'a [Vec<String>],
    pub pushed: &'a [Vec<String>],
    pub interface: &'a str,
    pub tunnel_has_ipv6: bool,
    pub block_ipv6: bool,
}

pub fn resolve(inputs: &Inputs) -> NetConfig {
    let s = inputs.settings;
    let mut info = DnsInfo::default();
    for server in &s.dns_servers {
        push_unique(&mut info.servers, server);
    }
    for domain in &s.dns_domains {
        push_unique(&mut info.domains, domain);
    }
    for wins in &s.wins_servers {
        push_unique(&mut info.wins, wins);
    }
    collect(inputs.config, &mut info, true);
    collect(inputs.pushed, &mut info, s.use_pushed_dns);
    if matches!(s.redirect, Redirect::All | Redirect::Ipv4) {
        info.redirect = true;
    }

    let mode = match s.dns_mode {
        DnsMode::Disabled => None,
        _ if info.servers.is_empty() => None,
        DnsMode::Full => Some(ResolvedDns::Full),
        DnsMode::Split => Some(ResolvedDns::Split),
        DnsMode::Automatic if info.redirect => Some(ResolvedDns::Full),
        DnsMode::Automatic => Some(ResolvedDns::Split),
    };

    let mut search = info.search.clone();
    for d in &info.domains {
        push_unique(&mut search, d);
    }
    let mut match_domains = info.domains.clone();
    if mode == Some(ResolvedDns::Split) {
        for d in &info.search {
            push_unique(&mut match_domains, d);
        }
    }

    let proxies: &InterfaceProxies = &s.interface_proxies;
    let to_pair = |hp: &Option<HostPort>| hp.as_ref().map(|hp| (hp.host.clone(), hp.port));
    let full = mode == Some(ResolvedDns::Full);

    NetConfig {
        interface: inputs.interface.to_string(),
        dns: mode,
        servers: if mode.is_some() { info.servers } else { Vec::new() },
        match_domains,
        search_domains: search,
        wins: if s.use_pushed_wins || !s.wins_servers.is_empty() {
            info.wins
        } else {
            Vec::new()
        },
        proxy_auto: proxies.automatic.clone().or(info.proxy_auto).filter(|_| full),
        proxy_http: to_pair(&proxies.http).or(info.proxy_http).filter(|_| full),
        proxy_https: to_pair(&proxies.https).or(info.proxy_https).filter(|_| full),
        proxy_socks: to_pair(&proxies.socks).filter(|_| full),
        block_ipv6: inputs.block_ipv6 && !inputs.tunnel_has_ipv6,
    }
}

pub fn split_dns_warning(config: &NetConfig) -> bool {
    config.dns == Some(ResolvedDns::Split) && config.match_domains.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUSH: &str = "PUSH: Received control message: 'PUSH_REPLY,redirect-gateway def1,dhcp-option DNS 10.8.0.1,dhcp-option DOMAIN corp.example,route-gateway 10.8.0.1,topology subnet,ping 10,ifconfig 10.8.0.2 255.255.255.0,peer-id 0,cipher AES-256-GCM'";

    fn settings(text: &str) -> Settings {
        Settings::from_document(&Document::parse(text).unwrap())
    }

    #[test]
    fn parses_push_reply() {
        let opts = parse_push_reply(PUSH).unwrap();
        assert_eq!(opts[0], ["redirect-gateway", "def1"]);
        assert_eq!(opts[1], ["dhcp-option", "DNS", "10.8.0.1"]);
        assert_eq!(opts.last().unwrap(), &["cipher", "AES-256-GCM"]);
    }

    #[test]
    fn automatic_full_when_redirected() {
        let pushed = parse_push_reply(PUSH).unwrap();
        let s = settings("client\n");
        let cfg = resolve(&Inputs {
            settings: &s,
            config: &[],
            pushed: &pushed,
            interface: "utun4",
            tunnel_has_ipv6: false,
            block_ipv6: true,
        });
        assert_eq!(cfg.dns, Some(ResolvedDns::Full));
        assert_eq!(cfg.servers, ["10.8.0.1"]);
        assert_eq!(cfg.search_domains, ["corp.example"]);
        assert!(cfg.block_ipv6);
    }

    #[test]
    fn automatic_split_and_overrides() {
        let pushed = vec![
            vec!["dhcp-option".into(), "DNS".into(), "10.8.0.1".into()],
            vec![
                "dns".into(),
                "server".into(),
                "0".into(),
                "address".into(),
                "[fd00::1]:53".into(),
                "10.8.0.9:5353".into(),
            ],
            vec!["dns".into(), "search-domains".into(), "lab.example".into()],
        ];
        let s = settings("#fluidity dnsserver 192.168.1.1\n#fluidity dnsdomain internal\n");
        let cfg = resolve(&Inputs {
            settings: &s,
            config: &[],
            pushed: &pushed,
            interface: "tun0",
            tunnel_has_ipv6: true,
            block_ipv6: true,
        });
        assert_eq!(cfg.dns, Some(ResolvedDns::Split));
        assert_eq!(cfg.servers, ["192.168.1.1", "10.8.0.1", "fd00::1", "10.8.0.9"]);
        assert_eq!(cfg.match_domains, ["internal", "lab.example"]);
        assert!(!cfg.block_ipv6);
    }

    #[test]
    fn ignores_pushed_dns_and_disables_without_servers() {
        let pushed = parse_push_reply(PUSH).unwrap();
        let s = settings("#fluidity usepeerdns false\n");
        let cfg = resolve(&Inputs {
            settings: &s,
            config: &[],
            pushed: &pushed,
            interface: "tun0",
            tunnel_has_ipv6: false,
            block_ipv6: false,
        });
        assert_eq!(cfg.dns, None);
        assert!(cfg.is_empty());
    }

    #[test]
    fn interface_proxies_need_full_dns() {
        let s = settings("#fluidity dns split\n#fluidity dnsserver 10.0.0.1\n#fluidity proxy-http p 3128\n");
        let cfg = resolve(&Inputs {
            settings: &s,
            config: &[],
            pushed: &[],
            interface: "tun0",
            tunnel_has_ipv6: false,
            block_ipv6: false,
        });
        assert!(cfg.proxy_http.is_none());
        assert!(split_dns_warning(&cfg));
    }
}
