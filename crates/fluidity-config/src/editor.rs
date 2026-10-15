use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Item};
use crate::settings::{DnsMode, Obfuscation, Redirect, Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    #[default]
    Udp,
    Tcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IpVersion {
    #[default]
    Automatic,
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Remote {
    pub host: String,
    pub port: u16,
    pub transport: Transport,
    pub ip_version: IpVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    #[default]
    Tun,
    Tap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    #[default]
    Tls,
    Pkcs11,
    Pkcs12,
    StaticKey,
    SystemIdentity,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExtraType {
    #[default]
    None,
    TlsAuth,
    TlsCrypt,
    TlsCryptV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FileField {
    pub name: String,
    pub data: Option<String>,
}

impl FileField {
    pub fn is_set(&self) -> bool {
        !self.name.is_empty() || self.data.is_some()
    }

    pub fn bytes(&self) -> Option<Vec<u8>> {
        STANDARD.decode(self.data.as_ref()?).ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Compression {
    #[default]
    Off,
    None,
    Lzo,
    Lz4,
    Lz4V2,
    Stub,
    StubV2,
    Adaptive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProxyType {
    #[default]
    None,
    Http,
    Socks,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProxyAuth {
    #[default]
    None,
    Basic,
    Ntlm,
    Automatic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Route {
    pub network: String,
    pub mask: String,
    pub gateway: String,
    pub metric: String,
    pub ipv6: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EditorModel {
    pub name: String,
    pub remotes: Vec<Remote>,
    pub remote_random: bool,
    pub server_timeout: Option<u32>,
    pub device: Device,
    pub dhcp: bool,
    pub ipv6: bool,
    pub auto_reconnect: bool,
    pub connect_on_launch: bool,

    pub auth_type: AuthType,
    pub user_pass: bool,
    pub ca: FileField,
    pub cert: FileField,
    pub key: FileField,
    pub pkcs12: FileField,
    pub static_key: FileField,
    pub static_key_direction: Option<u8>,
    pub pkcs11_providers: String,
    pub pkcs11_id: String,
    pub pkcs11_prompt: bool,
    pub system_identity: String,
    pub extra_type: ExtraType,
    pub extra_key: FileField,
    pub extra_direction: Option<u8>,

    pub ping: Option<u32>,
    pub ping_restart: Option<u32>,
    pub persist_tun: bool,
    pub persist_key: bool,
    pub persist_local_ip: bool,
    pub persist_remote_ip: bool,
    pub remote_cert_tls: bool,
    pub compression: Compression,
    pub nobind: bool,
    pub pull: bool,
    pub compatibility: String,

    pub redirect: Redirect,
    pub routes: Vec<Route>,
    pub default_gateway: String,
    pub dns_mode: String,
    pub dns_servers: Vec<String>,
    pub dns_domains: Vec<String>,
    pub ignore_pushed_dns: bool,
    pub shaper: Option<u32>,
    pub fragment: Option<u32>,
    pub tun_mtu: Option<u32>,
    pub inactive: Option<u32>,

    pub proxy_type: ProxyType,
    pub proxy_host: String,
    pub proxy_port: Option<u16>,
    pub proxy_auth: ProxyAuth,
    pub obfuscation: String,
    pub obfuscation_key: String,

    pub before_connect_script: String,
    pub connected_script: String,
    pub disconnected_script: String,
    pub extra_commands: String,
}

const EXTRA_META: &[&str] = &[
    "proxy-automatic",
    "proxy-http",
    "proxy-https",
    "proxy-socks",
    "static-challenge-password",
    "static-challenge-acknowledge",
    "system-identity-domain",
    "winsserver",
];

const MANAGED: &[&str] = &[
    "client",
    "remote",
    "proto",
    "port",
    "rport",
    "remote-random",
    "server-poll-timeout",
    "dev",
    "dev-type",
    "tun-ipv6",
    "auth-user-pass",
    "tls-client",
    "pull",
    "ca",
    "cert",
    "key",
    "pkcs12",
    "secret",
    "pkcs11-providers",
    "pkcs11-id",
    "pkcs11-id-management",
    "tls-auth",
    "tls-crypt",
    "tls-crypt-v2",
    "key-direction",
    "ping",
    "ping-restart",
    "keepalive",
    "persist-tun",
    "persist-key",
    "persist-local-ip",
    "persist-remote-ip",
    "remote-cert-tls",
    "ns-cert-type",
    "comp-lzo",
    "compress",
    "nobind",
    "redirect-gateway",
    "redirect-private",
    "route",
    "route-ipv6",
    "route-gateway",
    "shaper",
    "fragment",
    "tun-mtu",
    "inactive",
    "http-proxy",
    "socks-proxy",
    "http-proxy-retry",
    "socks-proxy-retry",
];

fn num<T: std::str::FromStr>(args: Option<&[String]>, i: usize) -> Option<T> {
    args?.get(i)?.parse().ok()
}

fn file_field(doc: &Document, files: &BTreeMap<String, Vec<u8>>, name: &str) -> FileField {
    if let Some(body) = doc.inline(name) {
        return FileField {
            name: String::new(),
            data: Some(STANDARD.encode(body)),
        };
    }
    let Some(file) = doc.directive(name).and_then(|a| a.first()) else {
        return FileField::default();
    };
    FileField {
        name: file.clone(),
        data: files.get(file).map(|d| STANDARD.encode(d)),
    }
}

fn parse_transport(proto: &str) -> (Transport, IpVersion) {
    let transport = if proto.starts_with("tcp") {
        Transport::Tcp
    } else {
        Transport::Udp
    };
    let version = if proto.contains('6') {
        IpVersion::Ipv6
    } else if proto.contains('4') {
        IpVersion::Ipv4
    } else {
        IpVersion::Automatic
    };
    (transport, version)
}

fn proto_string(transport: Transport, version: IpVersion) -> String {
    let base = match transport {
        Transport::Udp => "udp",
        Transport::Tcp => "tcp",
    };
    let suffix = match version {
        IpVersion::Automatic => "",
        IpVersion::Ipv4 => "4",
        IpVersion::Ipv6 => "6",
    };
    let client = if transport == Transport::Tcp { "-client" } else { "" };
    format!("{base}{suffix}{client}")
}

impl EditorModel {
    pub fn new_connection() -> Self {
        Self {
            name: "New Connection".into(),
            remotes: vec![Remote {
                host: String::new(),
                port: 1194,
                ..Remote::default()
            }],
            ipv6: true,
            auto_reconnect: true,
            user_pass: false,
            persist_tun: true,
            persist_key: true,
            remote_cert_tls: true,
            nobind: true,
            pull: true,
            dns_mode: "automatic".into(),
            ..Self::default()
        }
    }

    pub fn from_document(doc: &Document, files: &BTreeMap<String, Vec<u8>>) -> Self {
        let settings = Settings::from_document(doc);
        let default_proto = doc
            .directive("proto")
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or_default();
        let default_port: u16 = num(doc.directive("port"), 0)
            .or(num(doc.directive("rport"), 0))
            .unwrap_or(1194);
        let remotes = doc
            .directives("remote")
            .map(|args| {
                let proto = args.get(2).cloned().unwrap_or_else(|| default_proto.clone());
                let (transport, ip_version) = parse_transport(&proto);
                Remote {
                    host: args.first().cloned().unwrap_or_default(),
                    port: args.get(1).and_then(|p| p.parse().ok()).unwrap_or(default_port),
                    transport,
                    ip_version,
                }
            })
            .collect();

        let dev = doc
            .directive("dev")
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or_default();
        let device = if dev.starts_with("tap")
            || doc
                .directive("dev-type")
                .is_some_and(|a| a.first().is_some_and(|t| t == "tap"))
        {
            Device::Tap
        } else {
            Device::Tun
        };

        let auth_type = if settings.system_identity.is_some() {
            AuthType::SystemIdentity
        } else if doc.has("pkcs11-providers") {
            AuthType::Pkcs11
        } else if doc.has("pkcs12") {
            AuthType::Pkcs12
        } else if doc.has("secret") {
            AuthType::StaticKey
        } else if doc.has("ca") || doc.has("cert") || doc.has("tls-client") || doc.has("client") {
            AuthType::Tls
        } else {
            AuthType::None
        };

        let (extra_type, extra_name) = if doc.has("tls-crypt-v2") {
            (ExtraType::TlsCryptV2, "tls-crypt-v2")
        } else if doc.has("tls-crypt") {
            (ExtraType::TlsCrypt, "tls-crypt")
        } else if doc.has("tls-auth") {
            (ExtraType::TlsAuth, "tls-auth")
        } else {
            (ExtraType::None, "")
        };
        let extra_direction = if extra_type == ExtraType::TlsAuth {
            num(doc.directive("tls-auth"), 1).or(num(doc.directive("key-direction"), 0))
        } else {
            None
        };

        let keepalive = doc.directive("keepalive");
        let compression = if let Some(args) = doc.directive("compress") {
            match args.first().map(String::as_str) {
                None => Compression::Stub,
                Some("lzo") => Compression::Lzo,
                Some("lz4") => Compression::Lz4,
                Some("lz4-v2") => Compression::Lz4V2,
                Some("stub") => Compression::Stub,
                Some("stub-v2") => Compression::StubV2,
                _ => Compression::None,
            }
        } else if let Some(args) = doc.directive("comp-lzo") {
            match args.first().map(String::as_str) {
                Some("no") => Compression::None,
                Some("adaptive") => Compression::Adaptive,
                _ => Compression::Lzo,
            }
        } else {
            Compression::Off
        };

        let mut routes = Vec::new();
        for args in doc.directives("route") {
            routes.push(Route {
                network: args.first().cloned().unwrap_or_default(),
                mask: args.get(1).cloned().unwrap_or_default(),
                gateway: args.get(2).cloned().unwrap_or_default(),
                metric: args.get(3).cloned().unwrap_or_default(),
                ipv6: false,
            });
        }
        for args in doc.directives("route-ipv6") {
            routes.push(Route {
                network: args.first().cloned().unwrap_or_default(),
                mask: String::new(),
                gateway: args.get(1).cloned().unwrap_or_default(),
                metric: args.get(2).cloned().unwrap_or_default(),
                ipv6: true,
            });
        }

        let (proxy_type, proxy_args) = if let Some(args) = doc.directive("http-proxy") {
            (ProxyType::Http, Some(args))
        } else if let Some(args) = doc.directive("socks-proxy") {
            (ProxyType::Socks, Some(args))
        } else if settings.use_system_proxy {
            (ProxyType::System, None)
        } else {
            (ProxyType::None, None)
        };
        let proxy_auth = match proxy_args.and_then(|a| a.get(3)).map(String::as_str) {
            Some("basic") => ProxyAuth::Basic,
            Some("ntlm") | Some("ntlm2") => ProxyAuth::Ntlm,
            Some("auto") | Some("auto-nct") => ProxyAuth::Automatic,
            _ if proxy_type == ProxyType::Socks && proxy_args.is_some_and(|a| a.len() > 2) => ProxyAuth::Basic,
            _ => ProxyAuth::None,
        };

        let mut extra = Document::default();
        for item in &doc.items {
            match item {
                Item::Meta { key, .. } if EXTRA_META.contains(&key.as_str()) => extra.items.push(item.clone()),
                Item::Directive { name, .. } if !MANAGED.contains(&name.as_str()) => extra.items.push(item.clone()),
                Item::Inline { tag, .. }
                    if ![
                        "ca",
                        "cert",
                        "key",
                        "pkcs12",
                        "secret",
                        "tls-auth",
                        "tls-crypt",
                        "tls-crypt-v2",
                    ]
                    .contains(&tag.as_str()) =>
                {
                    extra.items.push(item.clone())
                }
                _ => {}
            }
        }

        Self {
            name: settings.name.clone(),
            remotes,
            remote_random: doc.has("remote-random"),
            server_timeout: num(doc.directive("server-poll-timeout"), 0),
            device,
            dhcp: settings.dhcp,
            ipv6: settings.ipv6,
            auto_reconnect: settings.auto_reconnect,
            connect_on_launch: settings.connect_on_launch,
            auth_type,
            user_pass: doc.has("auth-user-pass"),
            ca: file_field(doc, files, "ca"),
            cert: file_field(doc, files, "cert"),
            key: file_field(doc, files, "key"),
            pkcs12: file_field(doc, files, "pkcs12"),
            static_key: file_field(doc, files, "secret"),
            static_key_direction: num(doc.directive("secret"), 1),
            pkcs11_providers: doc
                .directive("pkcs11-providers")
                .map(|a| a.join(" "))
                .unwrap_or_default(),
            pkcs11_id: doc
                .directive("pkcs11-id")
                .and_then(|a| a.first().cloned())
                .unwrap_or_default(),
            pkcs11_prompt: doc.has("pkcs11-id-management"),
            system_identity: settings.system_identity.clone().unwrap_or_default(),
            extra_type,
            extra_key: if extra_name.is_empty() {
                FileField::default()
            } else {
                file_field(doc, files, extra_name)
            },
            extra_direction,
            ping: num(doc.directive("ping"), 0).or(num(keepalive, 0)),
            ping_restart: num(doc.directive("ping-restart"), 0).or(num(keepalive, 1)),
            persist_tun: doc.has("persist-tun"),
            persist_key: doc.has("persist-key"),
            persist_local_ip: doc.has("persist-local-ip"),
            persist_remote_ip: doc.has("persist-remote-ip"),
            remote_cert_tls: doc
                .directive("remote-cert-tls")
                .is_some_and(|a| a.first().is_some_and(|v| v == "server"))
                || doc.has("ns-cert-type"),
            compression,
            nobind: doc.has("nobind"),
            pull: doc.has("pull") || doc.has("client"),
            compatibility: settings.openvpn_version.clone().unwrap_or_default(),
            redirect: if doc.has("redirect-gateway") && settings.redirect == Redirect::Automatic {
                let args = doc.directive("redirect-gateway").unwrap_or_default();
                if args.iter().any(|a| a == "ipv6") && args.iter().any(|a| a == "!ipv4") {
                    Redirect::Ipv6
                } else if args.iter().any(|a| a == "ipv6") {
                    Redirect::All
                } else {
                    Redirect::Ipv4
                }
            } else {
                settings.redirect
            },
            routes,
            default_gateway: doc
                .directive("route-gateway")
                .and_then(|a| a.first().cloned())
                .unwrap_or_default(),
            dns_mode: settings.dns_mode.as_str().to_string(),
            dns_servers: settings.dns_servers.clone(),
            dns_domains: settings.dns_domains.clone(),
            ignore_pushed_dns: !settings.use_pushed_dns,
            shaper: num(doc.directive("shaper"), 0),
            fragment: num(doc.directive("fragment"), 0),
            tun_mtu: num(doc.directive("tun-mtu"), 0),
            inactive: num(doc.directive("inactive"), 0),
            proxy_type,
            proxy_host: proxy_args.and_then(|a| a.first().cloned()).unwrap_or_default(),
            proxy_port: proxy_args.and_then(|a| a.get(1)).and_then(|p| p.parse().ok()),
            proxy_auth,
            obfuscation: settings.obfuscation.map(|o| o.as_str().to_string()).unwrap_or_default(),
            obfuscation_key: settings.obfuscation_key.clone().unwrap_or_default(),
            before_connect_script: settings.scripts.before_connect.clone().unwrap_or_default(),
            connected_script: settings.scripts.connected.clone().unwrap_or_default(),
            disconnected_script: settings.scripts.disconnected.clone().unwrap_or_default(),
            extra_commands: extra.to_string(),
        }
    }

    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.name.trim().is_empty() {
            problems.push("Enter a connection name.".into());
        }
        if self.remotes.is_empty() || self.remotes.iter().any(|r| r.host.trim().is_empty()) {
            problems.push("Enter a server address.".into());
        }
        if self.remotes.iter().any(|r| r.port == 0) {
            problems.push("Enter a valid port.".into());
        }
        match self.auth_type {
            AuthType::Tls if !self.ca.is_set() => problems.push("Select a CA certificate.".into()),
            AuthType::Pkcs12 if !self.pkcs12.is_set() => problems.push("Select a PKCS#12 file.".into()),
            AuthType::StaticKey if !self.static_key.is_set() => problems.push("Select a static key file.".into()),
            AuthType::Pkcs11 if self.pkcs11_providers.trim().is_empty() => {
                problems.push("Select a PKCS#11 provider.".into())
            }
            AuthType::SystemIdentity if self.system_identity.trim().is_empty() => {
                problems.push("Select a system identity.".into())
            }
            _ => {}
        }
        if self.extra_type != ExtraType::None && !self.extra_key.is_set() {
            problems.push("Select the TLS key file.".into());
        }
        if matches!(self.proxy_type, ProxyType::Http | ProxyType::Socks)
            && (self.proxy_host.trim().is_empty() || self.proxy_port.is_none())
        {
            problems.push("Enter the proxy address and port.".into());
        }
        if !self.obfuscation.is_empty() && self.obfuscation.parse::<Obfuscation>().is_err() {
            problems.push("Select a valid obfuscation method.".into());
        }
        if let Err(e) = Document::parse(&self.extra_commands) {
            problems.push(format!("Advanced commands: {e}"));
        }
        problems
    }

    pub fn to_bundle(&self) -> Result<(Document, BTreeMap<String, Vec<u8>>), crate::Error> {
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let push = |doc: &mut Document, name: &str, args: &[&str]| {
            doc.push(name, args.iter().map(|s| s.to_string()).collect());
        };

        let mut doc = Document::default();
        let mut settings = Settings {
            name: self.name.trim().to_string(),
            auto_reconnect: self.auto_reconnect,
            connect_on_launch: self.connect_on_launch,
            dhcp: self.dhcp,
            ipv6: self.ipv6,
            dns_mode: self.dns_mode.parse().unwrap_or(DnsMode::Automatic),
            dns_servers: self.dns_servers.clone(),
            dns_domains: self.dns_domains.clone(),
            use_pushed_dns: !self.ignore_pushed_dns,
            use_pushed_wins: !self.ignore_pushed_dns,
            redirect: self.redirect,
            use_system_proxy: self.proxy_type == ProxyType::System,
            obfuscation: self.obfuscation.parse().ok(),
            obfuscation_key: Some(self.obfuscation_key.clone()).filter(|k| !k.is_empty()),
            system_identity: (self.auth_type == AuthType::SystemIdentity)
                .then(|| self.system_identity.clone())
                .filter(|s| !s.is_empty()),
            openvpn_version: Some(self.compatibility.clone()).filter(|c| !c.is_empty()),
            ..Settings::default()
        };
        settings.scripts.before_connect = Some(self.before_connect_script.clone()).filter(|s| !s.is_empty());
        settings.scripts.connected = Some(self.connected_script.clone()).filter(|s| !s.is_empty());
        settings.scripts.disconnected = Some(self.disconnected_script.clone()).filter(|s| !s.is_empty());
        let extra = Document::parse(&self.extra_commands)?;
        for item in &extra.items {
            if let Item::Meta { key, args } = item
                && EXTRA_META.contains(&key.as_str())
            {
                doc.items.push(Item::Meta {
                    key: key.clone(),
                    args: args.clone(),
                });
            }
        }
        let base = Settings::from_document(&doc);
        settings.interface_proxies = base.interface_proxies;
        settings.static_challenge_password = base.static_challenge_password;
        settings.static_challenge_acknowledge = base.static_challenge_acknowledge;
        settings.system_identity_domain = base.system_identity_domain;
        settings.wins_servers = base.wins_servers;
        settings.apply(&mut doc);

        push(&mut doc, "client", &[]);
        push(
            &mut doc,
            "dev",
            &[match self.device {
                Device::Tun => "tun",
                Device::Tap => "tap",
            }],
        );
        for remote in &self.remotes {
            let proto = proto_string(remote.transport, remote.ip_version);
            push(
                &mut doc,
                "remote",
                &[remote.host.trim(), &remote.port.to_string(), &proto],
            );
        }
        if self.remote_random {
            push(&mut doc, "remote-random", &[]);
        }
        if let Some(timeout) = self.server_timeout {
            push(&mut doc, "server-poll-timeout", &[&timeout.to_string()]);
        }
        if self.pull && !doc.has("client") {
            push(&mut doc, "pull", &[]);
        }

        let mut attach =
            |doc: &mut Document, directive: &str, field: &FileField, default_name: &str, extra: &[String]| {
                let name = if field.name.is_empty() {
                    default_name.to_string()
                } else {
                    std::path::Path::new(&field.name)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(default_name)
                        .to_string()
                };
                if let Some(bytes) = field.bytes() {
                    files.insert(name.clone(), bytes);
                }
                let mut args = vec![name];
                args.extend(extra.iter().cloned());
                doc.push(directive, args);
            };

        match self.auth_type {
            AuthType::Tls | AuthType::SystemIdentity => {
                if self.ca.is_set() {
                    attach(&mut doc, "ca", &self.ca, "ca.crt", &[]);
                }
                if self.auth_type == AuthType::Tls {
                    if self.cert.is_set() {
                        attach(&mut doc, "cert", &self.cert, "cert.crt", &[]);
                    }
                    if self.key.is_set() {
                        attach(&mut doc, "key", &self.key, "key.key", &[]);
                    }
                }
            }
            AuthType::Pkcs11 => {
                if self.ca.is_set() {
                    attach(&mut doc, "ca", &self.ca, "ca.crt", &[]);
                }
                doc.push(
                    "pkcs11-providers",
                    self.pkcs11_providers.split_whitespace().map(str::to_string).collect(),
                );
                if self.pkcs11_prompt || self.pkcs11_id.is_empty() {
                    push(&mut doc, "pkcs11-id-management", &[]);
                } else {
                    push(&mut doc, "pkcs11-id", &[&self.pkcs11_id]);
                }
            }
            AuthType::Pkcs12 => attach(&mut doc, "pkcs12", &self.pkcs12, "client.p12", &[]),
            AuthType::StaticKey => {
                let dir: Vec<String> = self.static_key_direction.map(|d| d.to_string()).into_iter().collect();
                attach(&mut doc, "secret", &self.static_key, "static.key", &dir);
            }
            AuthType::None => {}
        }
        match self.extra_type {
            ExtraType::None => {}
            ExtraType::TlsAuth => {
                let dir: Vec<String> = self.extra_direction.map(|d| d.to_string()).into_iter().collect();
                attach(&mut doc, "tls-auth", &self.extra_key, "ta.key", &dir);
            }
            ExtraType::TlsCrypt => attach(&mut doc, "tls-crypt", &self.extra_key, "tls-crypt.key", &[]),
            ExtraType::TlsCryptV2 => attach(&mut doc, "tls-crypt-v2", &self.extra_key, "tls-crypt-v2.key", &[]),
        }
        if self.user_pass {
            push(&mut doc, "auth-user-pass", &[]);
        }

        if let Some(p) = self.ping {
            push(&mut doc, "ping", &[&p.to_string()]);
        }
        if let Some(p) = self.ping_restart {
            push(&mut doc, "ping-restart", &[&p.to_string()]);
        }
        for (flag, name) in [
            (self.persist_tun, "persist-tun"),
            (self.persist_key, "persist-key"),
            (self.persist_local_ip, "persist-local-ip"),
            (self.persist_remote_ip, "persist-remote-ip"),
            (self.nobind, "nobind"),
        ] {
            if flag {
                push(&mut doc, name, &[]);
            }
        }
        if self.remote_cert_tls {
            push(&mut doc, "remote-cert-tls", &["server"]);
        }
        match self.compression {
            Compression::Off => {}
            Compression::None => push(&mut doc, "comp-lzo", &["no"]),
            Compression::Adaptive => push(&mut doc, "comp-lzo", &["adaptive"]),
            Compression::Lzo => push(&mut doc, "compress", &["lzo"]),
            Compression::Lz4 => push(&mut doc, "compress", &["lz4"]),
            Compression::Lz4V2 => push(&mut doc, "compress", &["lz4-v2"]),
            Compression::Stub => push(&mut doc, "compress", &["stub"]),
            Compression::StubV2 => push(&mut doc, "compress", &["stub-v2"]),
        }

        match self.redirect {
            Redirect::Automatic => {}
            Redirect::All => push(&mut doc, "redirect-gateway", &["def1", "ipv6"]),
            Redirect::Ipv4 => push(&mut doc, "redirect-gateway", &["def1"]),
            Redirect::Ipv6 => push(&mut doc, "redirect-gateway", &["ipv6", "!ipv4"]),
        }
        if !self.default_gateway.trim().is_empty() {
            push(&mut doc, "route-gateway", &[self.default_gateway.trim()]);
        }
        for route in &self.routes {
            if route.network.trim().is_empty() {
                continue;
            }
            if route.ipv6 {
                let mut args = vec![route.network.trim().to_string()];
                if !route.gateway.is_empty() || !route.metric.is_empty() {
                    args.push(if route.gateway.is_empty() {
                        "default".into()
                    } else {
                        route.gateway.clone()
                    });
                }
                if !route.metric.is_empty() {
                    args.push(route.metric.clone());
                }
                doc.push("route-ipv6", args);
            } else {
                let mut args = vec![route.network.trim().to_string()];
                let mask = if route.mask.is_empty() {
                    "255.255.255.255".to_string()
                } else {
                    route.mask.clone()
                };
                args.push(mask);
                if !route.gateway.is_empty() || !route.metric.is_empty() {
                    args.push(if route.gateway.is_empty() {
                        "vpn_gateway".into()
                    } else {
                        route.gateway.clone()
                    });
                }
                if !route.metric.is_empty() {
                    args.push(route.metric.clone());
                }
                doc.push("route", args);
            }
        }
        for (value, name) in [
            (self.shaper, "shaper"),
            (self.fragment, "fragment"),
            (self.tun_mtu, "tun-mtu"),
            (self.inactive, "inactive"),
        ] {
            if let Some(v) = value {
                push(&mut doc, name, &[&v.to_string()]);
            }
        }

        if matches!(self.proxy_type, ProxyType::Http | ProxyType::Socks) {
            let name = if self.proxy_type == ProxyType::Http {
                "http-proxy"
            } else {
                "socks-proxy"
            };
            let mut args = vec![
                self.proxy_host.trim().to_string(),
                self.proxy_port.unwrap_or(8080).to_string(),
            ];
            match (self.proxy_type, self.proxy_auth) {
                (ProxyType::Http, ProxyAuth::Basic) => args.extend(["stdin".into(), "basic".into()]),
                (ProxyType::Http, ProxyAuth::Ntlm) => args.extend(["stdin".into(), "ntlm2".into()]),
                (ProxyType::Http, ProxyAuth::Automatic) => args.extend(["auto".into()]),
                (ProxyType::Socks, ProxyAuth::Basic) => args.push("stdin".into()),
                _ => {}
            }
            doc.push(name, args);
        }

        for item in extra.items {
            if !matches!(item, Item::Meta { .. }) {
                doc.items.push(item);
            }
        }
        Ok((doc, files))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"#fluidity name "Office"
#fluidity dns split
#fluidity dnsdomain corp.example
#fluidity proxy-http proxy.corp 3128
client
dev tun
proto udp
remote vpn1.example.com 1194
remote vpn2.example.com 443 tcp-client
server-poll-timeout 15
ca ca.crt
cert client.crt
key client.key
tls-auth ta.key 1
auth-user-pass
keepalive 10 60
persist-tun
persist-key
remote-cert-tls server
comp-lzo no
nobind
route 10.1.0.0 255.255.0.0
route-ipv6 fd00::/64
tun-mtu 1400
http-proxy proxy.example 8080 stdin basic
cipher AES-256-GCM
verb 4
<connection>
remote x 1
</connection>
"#;

    fn files() -> BTreeMap<String, Vec<u8>> {
        ["ca.crt", "client.crt", "client.key", "ta.key"]
            .iter()
            .map(|n| (n.to_string(), format!("{n} data").into_bytes()))
            .collect()
    }

    #[test]
    fn reads_every_tab() {
        let m = EditorModel::from_document(&Document::parse(SAMPLE).unwrap(), &files());
        assert_eq!(m.name, "Office");
        assert_eq!(m.remotes.len(), 2);
        assert_eq!(m.remotes[1].transport, Transport::Tcp);
        assert_eq!(m.remotes[1].port, 443);
        assert_eq!(m.server_timeout, Some(15));
        assert_eq!(m.auth_type, AuthType::Tls);
        assert!(m.user_pass);
        assert_eq!(m.ca.bytes().unwrap(), b"ca.crt data");
        assert_eq!(m.extra_type, ExtraType::TlsAuth);
        assert_eq!(m.extra_direction, Some(1));
        assert_eq!((m.ping, m.ping_restart), (Some(10), Some(60)));
        assert_eq!(m.compression, Compression::None);
        assert_eq!(m.routes.len(), 2);
        assert_eq!(m.tun_mtu, Some(1400));
        assert_eq!(m.proxy_type, ProxyType::Http);
        assert_eq!(m.proxy_auth, ProxyAuth::Basic);
        assert_eq!(m.dns_mode, "split");
        assert!(m.extra_commands.contains("cipher AES-256-GCM"));
        assert!(m.extra_commands.contains("<connection>"));
        assert!(!m.extra_commands.contains("remote vpn1"));
        assert!(m.validate().is_empty(), "{:?}", m.validate());
    }

    #[test]
    fn writes_equivalent_config() {
        let m = EditorModel::from_document(&Document::parse(SAMPLE).unwrap(), &files());
        let (doc, out_files) = m.to_bundle().unwrap();
        let again = EditorModel::from_document(&doc, &out_files);
        let mut expected = m.clone();
        expected.ping = Some(10);
        assert_eq!(again, expected, "\n{doc}");
        assert_eq!(doc.meta("proxy-http"), Some("proxy.corp"));
        assert_eq!(out_files["ta.key"], b"ta.key data");
    }

    #[test]
    fn new_connection_requires_server_and_ca() {
        let m = EditorModel::new_connection();
        assert_eq!(m.validate(), ["Enter a server address.", "Select a CA certificate."]);
    }
}
