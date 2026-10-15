use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use fluidity_config::Document;
use fluidity_config::settings::Obfuscation;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

pub const AUTH_FILE: &str = "obfs-auth.txt";

fn binary_name() -> &'static str {
    if cfg!(windows) { "lyrebird.exe" } else { "lyrebird" }
}

pub fn discover() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("FLUIDITY_LYREBIRD") {
        return Some(PathBuf::from(path));
    }
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    [
        exe_dir.join(binary_name()),
        exe_dir.join("../Resources").join(binary_name()),
        exe_dir.join("../lib/fluidity").join(binary_name()),
        PathBuf::from("/Library/Application Support/Fluidity").join(binary_name()),
        PathBuf::from("/usr/lib/fluidity").join(binary_name()),
        PathBuf::from("/usr/bin").join(binary_name()),
    ]
    .into_iter()
    .find(|p| p.is_file() && crate::openvpn::trusted(p))
}

pub fn transport_args(method: Obfuscation, key: Option<&str>) -> String {
    let key = key.unwrap_or_default().trim();
    if key.contains('=') {
        return key.to_string();
    }
    match method {
        Obfuscation::Obfs4 if !key.is_empty() => format!("cert={key};iat-mode=0"),
        Obfuscation::ScrambleSuit if !key.is_empty() => format!("password={key}"),
        _ => String::new(),
    }
}

pub fn socks_credentials(args: &str) -> (String, String) {
    let split = if args.len() > 255 {
        255
    } else {
        args.char_indices().last().map(|(i, _)| i).unwrap_or(0).max(1)
    };
    let split = (1..=split).rev().find(|i| args.is_char_boundary(*i)).unwrap_or(args.len());
    let (user, pass) = args.split_at(split);
    (user.to_string(), pass.to_string())
}

pub fn parse_cmethod(line: &str, method: &str) -> Option<u16> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "CMETHOD" || parts.next()? != method || parts.next()? != "socks5" {
        return None;
    }
    parts.next()?.rsplit(':').next()?.parse().ok()
}

pub fn rewrite(document: &Document, port: u16, auth: bool) -> Document {
    let mut doc = document.clone();
    for name in ["socks-proxy", "http-proxy", "http-proxy-option", "proto"] {
        doc.remove(name);
    }
    let remotes: Vec<Vec<String>> = doc.directives("remote").map(|a| a.to_vec()).collect();
    doc.remove("remote");
    for mut args in remotes {
        args.truncate(2);
        args.push("tcp-client".into());
        doc.push("remote", args);
    }
    doc.push("proto", vec!["tcp-client".into()]);
    let mut proxy = vec!["127.0.0.1".to_string(), port.to_string()];
    if auth {
        proxy.push(AUTH_FILE.into());
    }
    doc.push("socks-proxy", proxy);
    doc
}

pub async fn start(binary: &Path, method: Obfuscation, state_dir: &Path) -> Result<(Child, u16), String> {
    let name = method.as_str();
    let mut child = Command::new(binary)
        .env("TOR_PT_MANAGED_TRANSPORT_VER", "1")
        .env("TOR_PT_CLIENT_TRANSPORTS", name)
        .env("TOR_PT_STATE_LOCATION", state_dir)
        .env("TOR_PT_EXIT_ON_STDIN_CLOSE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Cannot start lyrebird: {e}"))?;
    let stdout = child.stdout.take().ok_or("lyrebird has no output")?;
    let mut lines = BufReader::new(stdout).lines();
    let port = tokio::time::timeout(Duration::from_secs(10), async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(port) = parse_cmethod(&line, name) {
                return Ok(port);
            }
            if line.starts_with("CMETHOD-ERROR") || line.starts_with("ENV-ERROR") || line.starts_with("VERSION-ERROR") {
                return Err(format!("lyrebird: {line}"));
            }
        }
        Err("lyrebird exited during startup".to_string())
    })
    .await
    .map_err(|_| "lyrebird did not start".to_string())??;
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    Ok((child, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_transport_args() {
        assert_eq!(transport_args(Obfuscation::Obfs4, Some("abc")), "cert=abc;iat-mode=0");
        assert_eq!(transport_args(Obfuscation::Obfs4, Some("cert=x;iat-mode=1")), "cert=x;iat-mode=1");
        assert_eq!(transport_args(Obfuscation::Obfs3, None), "");
        assert_eq!(transport_args(Obfuscation::ScrambleSuit, Some("pw")), "password=pw");
    }

    #[test]
    fn splits_credentials() {
        let (u, p) = socks_credentials("cert=abc;iat-mode=0");
        assert_eq!(format!("{u}{p}"), "cert=abc;iat-mode=0");
        assert!(!p.is_empty());
        let long = "x".repeat(300);
        let (u, p) = socks_credentials(&long);
        assert_eq!(u.len(), 255);
        assert_eq!(p.len(), 45);
    }

    #[test]
    fn parses_cmethod() {
        assert_eq!(parse_cmethod("CMETHOD obfs4 socks5 127.0.0.1:41234", "obfs4"), Some(41234));
        assert_eq!(parse_cmethod("CMETHOD obfs3 socks5 127.0.0.1:1", "obfs4"), None);
        assert_eq!(parse_cmethod("CMETHODS DONE", "obfs4"), None);
    }

    #[test]
    fn rewrites_for_socks() {
        let doc = Document::parse("remote a.example 443 udp\nproto udp\ndev tun\n").unwrap();
        let out = rewrite(&doc, 9000, true).to_string();
        assert!(out.contains("remote a.example 443 tcp-client"));
        assert!(out.contains("proto tcp-client"));
        assert!(out.contains("socks-proxy 127.0.0.1 9000 obfs-auth.txt"));
        assert!(!out.contains("udp"));
    }
}
