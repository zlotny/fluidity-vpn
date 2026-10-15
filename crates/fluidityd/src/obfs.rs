use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use fluidity_config::Document;
use fluidity_config::settings::Obfuscation;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

pub struct Transport {
    child: Child,
    shim: JoinHandle<()>,
    pub port: u16,
}

impl Transport {
    pub async fn stop(mut self) {
        self.shim.abort();
        let _ = self.child.kill().await;
    }
}

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

pub fn auth_request(args: &str) -> Vec<u8> {
    let bytes = args.as_bytes();
    let (user, pass) = bytes.split_at(bytes.len().min(255));
    let pass: &[u8] = if pass.is_empty() {
        &[0]
    } else {
        &pass[..pass.len().min(255)]
    };
    let mut msg = vec![1, user.len() as u8];
    msg.extend_from_slice(user);
    msg.push(pass.len() as u8);
    msg.extend_from_slice(pass);
    msg
}

pub fn parse_cmethod(line: &str, method: &str) -> Option<u16> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "CMETHOD" || parts.next()? != method || parts.next()? != "socks5" {
        return None;
    }
    parts.next()?.rsplit(':').next()?.parse().ok()
}

pub fn rewrite(document: &Document, port: u16) -> Document {
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
    doc.push("socks-proxy", vec!["127.0.0.1".to_string(), port.to_string()]);
    doc
}

pub async fn start(binary: &Path, method: Obfuscation, state_dir: &Path, args: String) -> Result<Transport, String> {
    let (child, upstream) = spawn(binary, method, state_dir).await?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("Cannot open obfuscation port: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let shim = tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let args = args.clone();
            tokio::spawn(async move {
                if let Ok((mut client, mut server)) = bridge(client, upstream, &args).await {
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
                }
            });
        }
    });
    Ok(Transport { child, shim, port })
}

async fn read_request<R: AsyncRead + Unpin>(stream: &mut R) -> std::io::Result<Vec<u8>> {
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await?;
    let addr_len = match head[3] {
        1 => 4,
        4 => 16,
        3 => stream.read_u8().await? as usize,
        _ => return Err(std::io::Error::other("unsupported address type")),
    };
    let mut request = head.to_vec();
    if head[3] == 3 {
        request.push(addr_len as u8);
    }
    let mut rest = vec![0u8; addr_len + 2];
    stream.read_exact(&mut rest).await?;
    request.extend(rest);
    Ok(request)
}

async fn bridge(mut client: TcpStream, upstream: u16, args: &str) -> std::io::Result<(TcpStream, TcpStream)> {
    let fail = || std::io::Error::other("SOCKS negotiation failed");
    let mut greeting = [0u8; 2];
    client.read_exact(&mut greeting).await?;
    let mut methods = vec![0u8; greeting[1] as usize];
    client.read_exact(&mut methods).await?;
    if greeting[0] != 5 || !methods.contains(&0) {
        client.write_all(&[5, 0xff]).await?;
        return Err(fail());
    }
    client.write_all(&[5, 0]).await?;
    let request = read_request(&mut client).await?;
    let mut server = TcpStream::connect(("127.0.0.1", upstream)).await?;
    let auth = !args.is_empty();
    server.write_all(&[5, 1, if auth { 2 } else { 0 }]).await?;
    let mut reply = [0u8; 2];
    server.read_exact(&mut reply).await?;
    if reply != [5, if auth { 2 } else { 0 }] {
        return Err(fail());
    }
    if auth {
        server.write_all(&auth_request(args)).await?;
        server.read_exact(&mut reply).await?;
        if reply[1] != 0 {
            return Err(fail());
        }
    }
    server.write_all(&request).await?;
    Ok((client, server))
}

async fn spawn(binary: &Path, method: Obfuscation, state_dir: &Path) -> Result<(Child, u16), String> {
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
        assert_eq!(
            transport_args(Obfuscation::Obfs4, Some("cert=x;iat-mode=1")),
            "cert=x;iat-mode=1"
        );
        assert_eq!(transport_args(Obfuscation::Obfs3, None), "");
        assert_eq!(transport_args(Obfuscation::ScrambleSuit, Some("pw")), "password=pw");
    }

    #[test]
    fn builds_auth_request() {
        assert_eq!(auth_request("ab"), b"\x01\x02ab\x01\x00");
        let long = "x".repeat(300);
        let msg = auth_request(&long);
        assert_eq!(msg[1], 255);
        assert_eq!(msg[257], 45);
        assert_eq!(msg.len(), 2 + 255 + 1 + 45);
    }

    #[test]
    fn parses_cmethod() {
        assert_eq!(
            parse_cmethod("CMETHOD obfs4 socks5 127.0.0.1:41234", "obfs4"),
            Some(41234)
        );
        assert_eq!(parse_cmethod("CMETHOD obfs3 socks5 127.0.0.1:1", "obfs4"), None);
        assert_eq!(parse_cmethod("CMETHODS DONE", "obfs4"), None);
    }

    #[test]
    fn rewrites_for_socks() {
        let doc = Document::parse("remote a.example 443 udp\nproto udp\ndev tun\n").unwrap();
        let out = rewrite(&doc, 9000).to_string();
        assert!(out.contains("remote a.example 443 tcp-client"));
        assert!(out.contains("proto tcp-client"));
        assert!(out.contains("socks-proxy 127.0.0.1 9000\n"));
        assert!(!out.contains("udp"));
    }
}
