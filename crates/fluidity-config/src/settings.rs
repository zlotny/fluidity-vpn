use std::str::FromStr;

use crate::document::{Document, Item};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DnsMode {
    #[default]
    Automatic,
    Full,
    Split,
    Disabled,
}

impl DnsMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Full => "full",
            Self::Split => "split",
            Self::Disabled => "off",
        }
    }
}

impl FromStr for DnsMode {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s.to_ascii_lowercase().as_str() {
            "automatic" | "auto" => Ok(Self::Automatic),
            "full" => Ok(Self::Full),
            "split" => Ok(Self::Split),
            "off" | "disabled" | "disable" | "none" => Ok(Self::Disabled),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Redirect {
    #[default]
    Automatic,
    All,
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obfuscation {
    Obfs2,
    Obfs3,
    Obfs4,
    ScrambleSuit,
}

impl Obfuscation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Obfs2 => "obfs2",
            Self::Obfs3 => "obfs3",
            Self::Obfs4 => "obfs4",
            Self::ScrambleSuit => "scramblesuit",
        }
    }
}

impl FromStr for Obfuscation {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s.to_ascii_lowercase().as_str() {
            "obfs2" => Ok(Self::Obfs2),
            "obfs3" => Ok(Self::Obfs3),
            "obfs4" => Ok(Self::Obfs4),
            "scramblesuit" => Ok(Self::ScrambleSuit),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPort {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InterfaceProxies {
    pub automatic: Option<String>,
    pub http: Option<HostPort>,
    pub https: Option<HostPort>,
    pub socks: Option<HostPort>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticChallengePassword {
    pub text: String,
    pub delimiter: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scripts {
    pub before_connect: Option<String>,
    pub connected: Option<String>,
    pub disconnected: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub name: String,
    pub auto_reconnect: bool,
    pub connect_on_launch: bool,
    pub dhcp: bool,
    pub ipv6: bool,
    pub dns_mode: DnsMode,
    pub dns_servers: Vec<String>,
    pub dns_domains: Vec<String>,
    pub use_pushed_dns: bool,
    pub wins_servers: Vec<String>,
    pub use_pushed_wins: bool,
    pub redirect: Redirect,
    pub interface_proxies: InterfaceProxies,
    pub use_system_proxy: bool,
    pub obfuscation: Option<Obfuscation>,
    pub obfuscation_key: Option<String>,
    pub static_challenge_password: Option<StaticChallengePassword>,
    pub static_challenge_acknowledge: Option<String>,
    pub system_identity: Option<String>,
    pub system_identity_domain: Option<String>,
    pub openvpn_version: Option<String>,
    pub scripts: Scripts,
}

fn host_port(args: &[String]) -> Option<HostPort> {
    Some(HostPort {
        host: args.first()?.clone(),
        port: args.get(1)?.parse().ok()?,
    })
}

fn split_list(values: Vec<String>) -> Vec<String> {
    values
        .iter()
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect()
}

impl Settings {
    pub fn from_document(doc: &Document) -> Self {
        let flag = |key: &str, default: bool| doc.meta_bool(key).unwrap_or(default);
        let text = |key: &str| doc.meta(key).filter(|v| !v.is_empty()).map(str::to_string);
        let pair = |key: &str| doc.metas(key).next().and_then(host_port);
        let redirect = match doc.meta("redirect") {
            Some("all") => Redirect::All,
            Some("ipv4") => Redirect::Ipv4,
            Some("ipv6") => Redirect::Ipv6,
            _ => Redirect::Automatic,
        };
        Self {
            name: text("name").unwrap_or_default(),
            auto_reconnect: flag("autoreconnect", true),
            connect_on_launch: flag("startonopen", false),
            dhcp: flag("dhcp", false),
            ipv6: flag("ipv6", true),
            dns_mode: doc.meta("dns").and_then(|m| m.parse().ok()).unwrap_or_default(),
            dns_servers: split_list(doc.meta_values("dnsserver")),
            dns_domains: split_list(doc.meta_values("dnsdomain")),
            use_pushed_dns: flag("usepeerdns", true),
            wins_servers: split_list(doc.meta_values("winsserver")),
            use_pushed_wins: flag("usepeerwins", true),
            redirect,
            interface_proxies: InterfaceProxies {
                automatic: text("proxy-automatic"),
                http: pair("proxy-http"),
                https: pair("proxy-https"),
                socks: pair("proxy-socks"),
            },
            use_system_proxy: flag("systemproxy", false),
            obfuscation: doc.meta("obfs").and_then(|m| m.parse().ok()),
            obfuscation_key: text("obfskey"),
            static_challenge_password: doc.metas("static-challenge-password").next().and_then(|args| {
                Some(StaticChallengePassword {
                    text: args.first()?.clone(),
                    delimiter: args.get(1).cloned().unwrap_or_default(),
                })
            }),
            static_challenge_acknowledge: doc
                .metas("static-challenge-acknowledge")
                .next()
                .filter(|a| !a.is_empty())
                .map(|a| a.join(" ")),
            system_identity: text("system-identity"),
            system_identity_domain: text("system-identity-domain"),
            openvpn_version: text("openvpn-version"),
            scripts: Scripts {
                before_connect: text("preconnectscript"),
                connected: text("connectedscript"),
                disconnected: text("disconnectedscript"),
            },
        }
    }

    pub fn apply(&self, doc: &mut Document) {
        let bool_str = |b: bool| if b { "true" } else { "false" };
        doc.set_meta("name", &[&self.name]);
        doc.set_meta("protocol", &["openvpn"]);
        doc.set_meta("autoreconnect", &[bool_str(self.auto_reconnect)]);
        doc.set_meta("startonopen", &[bool_str(self.connect_on_launch)]);
        doc.set_meta("dhcp", &[bool_str(self.dhcp)]);
        doc.set_meta("ipv6", &[bool_str(self.ipv6)]);
        doc.set_meta("dns", &[self.dns_mode.as_str()]);
        set_list(doc, "dnsserver", &self.dns_servers);
        set_list(doc, "dnsdomain", &self.dns_domains);
        doc.set_meta("usepeerdns", &[bool_str(self.use_pushed_dns)]);
        set_list(doc, "winsserver", &self.wins_servers);
        doc.set_meta("usepeerwins", &[bool_str(self.use_pushed_wins)]);
        let redirect = match self.redirect {
            Redirect::Automatic => None,
            Redirect::All => Some("all"),
            Redirect::Ipv4 => Some("ipv4"),
            Redirect::Ipv6 => Some("ipv6"),
        };
        set_opt(doc, "redirect", redirect.map(|r| vec![r.to_string()]));
        let proxies = &self.interface_proxies;
        set_opt(doc, "proxy-automatic", proxies.automatic.clone().map(|u| vec![u]));
        for (key, value) in [
            ("proxy-http", &proxies.http),
            ("proxy-https", &proxies.https),
            ("proxy-socks", &proxies.socks),
        ] {
            set_opt(
                doc,
                key,
                value.as_ref().map(|hp| vec![hp.host.clone(), hp.port.to_string()]),
            );
        }
        set_opt(doc, "systemproxy", self.use_system_proxy.then(|| vec!["true".into()]));
        set_opt(doc, "obfs", self.obfuscation.map(|o| vec![o.as_str().into()]));
        set_opt(doc, "obfskey", self.obfuscation_key.clone().map(|k| vec![k]));
        set_opt(
            doc,
            "static-challenge-password",
            self.static_challenge_password.as_ref().map(|s| {
                let mut v = vec![s.text.clone()];
                if !s.delimiter.is_empty() {
                    v.push(s.delimiter.clone());
                }
                v
            }),
        );
        set_opt(
            doc,
            "static-challenge-acknowledge",
            self.static_challenge_acknowledge.clone().map(|t| vec![t]),
        );
        set_opt(doc, "system-identity", self.system_identity.clone().map(|v| vec![v]));
        set_opt(
            doc,
            "system-identity-domain",
            self.system_identity_domain.clone().map(|v| vec![v]),
        );
        set_opt(doc, "openvpn-version", self.openvpn_version.clone().map(|v| vec![v]));
        set_opt(
            doc,
            "preconnectscript",
            self.scripts.before_connect.clone().map(|v| vec![v]),
        );
        set_opt(doc, "connectedscript", self.scripts.connected.clone().map(|v| vec![v]));
        set_opt(
            doc,
            "disconnectedscript",
            self.scripts.disconnected.clone().map(|v| vec![v]),
        );
    }
}

fn set_list(doc: &mut Document, key: &str, values: &[String]) {
    let refs: Vec<&str> = values.iter().map(String::as_str).collect();
    doc.set_meta(key, &refs);
}

fn set_opt(doc: &mut Document, key: &str, args: Option<Vec<String>>) {
    match args {
        Some(args) => {
            let position = doc
                .items
                .iter()
                .position(|i| matches!(i, Item::Meta { key: k, .. } if k == key));
            doc.items
                .retain(|i| !matches!(i, Item::Meta { key: k, .. } if k == key));
            let item = Item::Meta {
                key: key.to_string(),
                args,
            };
            match position {
                Some(p) => doc.items.insert(p.min(doc.items.len()), item),
                None => doc.items.push(item),
            }
        }
        None => doc
            .items
            .retain(|i| !matches!(i, Item::Meta { key: k, .. } if k == key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_defaults_and_values() {
        let doc = Document::parse(
            "#fluidity name \"Office\"\n#fluidity dns split\n#fluidity dnsdomain a.com, b.com\n#fluidity dnsdomain c.com\n#fluidity proxy-http proxy.local 3128\n#fluidity static-challenge-password \"OTP\" :\nclient\n",
        )
        .unwrap();
        let s = Settings::from_document(&doc);
        assert_eq!(s.name, "Office");
        assert_eq!(s.dns_mode, DnsMode::Split);
        assert_eq!(s.dns_domains, ["a.com", "b.com", "c.com"]);
        assert!(s.auto_reconnect && s.use_pushed_dns && !s.connect_on_launch);
        assert_eq!(
            s.interface_proxies.http,
            Some(HostPort {
                host: "proxy.local".into(),
                port: 3128
            })
        );
        assert_eq!(s.static_challenge_password.unwrap().delimiter, ":");
    }

    #[test]
    fn apply_round_trips() {
        let mut s = Settings {
            name: "Lab".into(),
            dns_mode: DnsMode::Full,
            dns_servers: vec!["10.0.0.1".into(), "10.0.0.2".into()],
            redirect: Redirect::Ipv4,
            obfuscation: Some(Obfuscation::Obfs4),
            obfuscation_key: Some("cert=abc".into()),
            ..Settings::default()
        };
        s.scripts.connected = Some("up.sh".into());
        let mut doc = Document::parse("client\nremote a 1194\n").unwrap();
        s.apply(&mut doc);
        let again = Settings::from_document(&Document::parse(&doc.to_string()).unwrap());
        assert_eq!(again, s);
        assert!(doc.has("client"));

        s.obfuscation = None;
        s.apply(&mut doc);
        assert!(doc.meta("obfs").is_none());
    }
}
