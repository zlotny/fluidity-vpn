use std::collections::VecDeque;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::event::{DynamicChallenge, Event};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("management interface closed")]
    Closed,
    #[error("OpenVPN rejected the command: {0}")]
    Rejected(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Success(String),
    Lines(Vec<String>),
}

impl Response {
    pub fn lines(&self) -> &[String] {
        match self {
            Self::Lines(lines) => lines,
            Self::Success(_) => &[],
        }
    }
}

type Reply = oneshot::Sender<Result<Response, Error>>;

struct Request {
    line: String,
    reply: Reply,
}

#[derive(Clone)]
pub struct Client {
    requests: mpsc::UnboundedSender<Request>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Hup,
    Term,
    Usr1,
    Usr2,
}

impl Signal {
    fn name(self) -> &'static str {
        match self {
            Self::Hup => "SIGHUP",
            Self::Term => "SIGTERM",
            Self::Usr1 => "SIGUSR1",
            Self::Usr2 => "SIGUSR2",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteAction {
    Accept,
    Skip,
    Modify { host: String, port: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyAction {
    None,
    Http { host: String, port: u16, nct: bool },
    Socks { host: String, port: u16 },
}

pub fn quote(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    for c in arg.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

pub fn static_challenge_password(password: &str, response: &str) -> String {
    format!("SCRV1:{}:{}", STANDARD.encode(password), STANDARD.encode(response))
}

impl Client {
    pub fn new<S>(stream: S) -> (Self, mpsc::UnboundedReceiver<Event>)
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (requests, request_rx) = mpsc::unbounded_channel();
        let (events, event_rx) = mpsc::unbounded_channel();
        tokio::spawn(run(stream, request_rx, events));
        (Self { requests }, event_rx)
    }

    #[cfg(unix)]
    pub async fn connect_unix(path: &Path) -> Result<(Self, mpsc::UnboundedReceiver<Event>), Error> {
        let stream = tokio::net::UnixStream::connect(path).await?;
        Ok(Self::new(stream))
    }

    pub async fn command(&self, line: impl Into<String>) -> Result<Response, Error> {
        let (reply, rx) = oneshot::channel();
        self.requests
            .send(Request {
                line: line.into(),
                reply,
            })
            .map_err(|_| Error::Closed)?;
        rx.await.map_err(|_| Error::Closed)?
    }

    pub async fn hold_release(&self) -> Result<Response, Error> {
        self.command("hold release").await
    }

    pub async fn state_on(&self) -> Result<Response, Error> {
        self.command("state on all").await
    }

    pub async fn log_on(&self) -> Result<Response, Error> {
        self.command("log on all").await
    }

    pub async fn echo_on(&self) -> Result<Response, Error> {
        self.command("echo on all").await
    }

    pub async fn bytecount(&self, seconds: u32) -> Result<Response, Error> {
        self.command(format!("bytecount {seconds}")).await
    }

    pub async fn username(&self, realm: &str, username: &str) -> Result<Response, Error> {
        self.command(format!("username {} {}", quote(realm), quote(username)))
            .await
    }

    pub async fn password(&self, realm: &str, password: &str) -> Result<Response, Error> {
        self.command(format!("password {} {}", quote(realm), quote(password)))
            .await
    }

    pub async fn credentials(&self, realm: &str, username: &str, password: &str) -> Result<(), Error> {
        self.username(realm, username).await?;
        self.password(realm, password).await?;
        Ok(())
    }

    pub async fn challenge_response(
        &self,
        realm: &str,
        challenge: &DynamicChallenge,
        answer: &str,
    ) -> Result<(), Error> {
        self.credentials(realm, &challenge.username, &challenge.response(answer))
            .await
    }

    pub async fn auth_token(&self, token: &str) -> Result<Response, Error> {
        self.password("Auth-Token", token).await
    }

    pub async fn signal(&self, signal: Signal) -> Result<Response, Error> {
        self.command(format!("signal {}", signal.name())).await
    }

    pub async fn remote(&self, action: RemoteAction) -> Result<Response, Error> {
        let line = match action {
            RemoteAction::Accept => "remote ACCEPT".to_string(),
            RemoteAction::Skip => "remote SKIP".to_string(),
            RemoteAction::Modify { host, port } => format!("remote MOD {} {port}", quote(&host)),
        };
        self.command(line).await
    }

    pub async fn proxy(&self, action: ProxyAction) -> Result<Response, Error> {
        let line = match action {
            ProxyAction::None => "proxy NONE".to_string(),
            ProxyAction::Http { host, port, nct } => {
                format!("proxy HTTP {} {port}{}", quote(&host), if nct { " nct" } else { "" })
            }
            ProxyAction::Socks { host, port } => format!("proxy SOCKS {} {port}", quote(&host)),
        };
        self.command(line).await
    }

    pub async fn need_ok(&self, name: &str, ok: bool) -> Result<Response, Error> {
        self.command(format!("needok {} {}", quote(name), if ok { "ok" } else { "cancel" }))
            .await
    }

    pub async fn need_str(&self, name: &str, value: &str) -> Result<Response, Error> {
        self.command(format!("needstr {} {}", quote(name), quote(value))).await
    }

    pub async fn pk_signature(&self, signature_b64: &str) -> Result<Response, Error> {
        self.multiline("pk-sig", signature_b64).await
    }

    pub async fn certificate(&self, pem: &str) -> Result<Response, Error> {
        self.multiline("certificate", pem).await
    }

    async fn multiline(&self, command: &str, body: &str) -> Result<Response, Error> {
        let mut line = String::from(command);
        for chunk in body.lines().filter(|l| !l.is_empty()) {
            line.push('\n');
            line.push_str(chunk);
        }
        line.push_str("\nEND");
        self.command(line).await
    }

    pub async fn version(&self, management_version: u32) -> Result<Response, Error> {
        self.command(format!("version {management_version}")).await
    }

    pub async fn pkcs11_ids(&self) -> Result<Vec<String>, Error> {
        let count = self.command("pkcs11-id-count").await?;
        let n = match &count {
            Response::Success(text) => text.trim().parse().unwrap_or(0),
            Response::Lines(_) => 0,
        };
        let mut ids = Vec::with_capacity(n);
        for i in 0..n {
            if let Response::Success(entry) = self.command(format!("pkcs11-id-get {i}")).await? {
                ids.push(entry);
            }
        }
        Ok(ids)
    }
}

struct Pending {
    queue: VecDeque<Reply>,
    lines: Option<Vec<String>>,
}

impl Pending {
    fn complete(&mut self, result: Result<Response, Error>) {
        self.lines = None;
        if let Some(reply) = self.queue.pop_front() {
            let _ = reply.send(result);
        }
    }

    fn response_line(&mut self, line: &str) {
        if self.lines.is_none() {
            if let Some(text) = line.strip_prefix("SUCCESS:") {
                return self.complete(Ok(Response::Success(text.trim().to_string())));
            }
            if let Some(text) = line.strip_prefix("ERROR:") {
                return self.complete(Err(Error::Rejected(text.trim().to_string())));
            }
        }
        if line == "END" {
            let lines = self.lines.take().unwrap_or_default();
            return self.complete(Ok(Response::Lines(lines)));
        }
        self.lines.get_or_insert_with(Vec::new).push(line.to_string());
    }
}

async fn run<S>(stream: S, mut requests: mpsc::UnboundedReceiver<Request>, events: mpsc::UnboundedSender<Event>)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read).lines();
    let mut pending = Pending {
        queue: VecDeque::new(),
        lines: None,
    };
    let mut updown: Option<(String, Vec<(String, String)>)> = None;
    let mut accepting = true;

    loop {
        tokio::select! {
            request = requests.recv(), if accepting => {
                let Some(request) = request else {
                    accepting = false;
                    continue;
                };
                let mut data = request.line.into_bytes();
                data.push(b'\n');
                match write.write_all(&data).await {
                    Ok(()) => pending.queue.push_back(request.reply),
                    Err(err) => {
                        let _ = request.reply.send(Err(Error::Io(err)));
                    }
                }
            }
            line = reader.next_line() => {
                let Ok(Some(line)) = line else { break };
                let line = line.trim_end_matches('\r');
                let Some(notification) = line.strip_prefix('>') else {
                    pending.response_line(line);
                    continue;
                };
                let (kind, payload) = notification.split_once(':').unwrap_or((notification, ""));
                if kind == "PKCS11ID-COUNT" || kind == "PKCS11ID-ENTRY" {
                    pending.complete(Ok(Response::Success(payload.to_string())));
                    continue;
                }
                if kind == "UPDOWN" {
                    match payload.strip_prefix("ENV,") {
                        Some("END") => {
                            if let Some((direction, env)) = updown.take() {
                                let _ = events.send(Event::UpDown { direction, env });
                            }
                        }
                        Some(pair) => {
                            if let Some((_, env)) = updown.as_mut() {
                                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                                env.push((k.to_string(), v.to_string()));
                            }
                        }
                        None => updown = Some((payload.to_string(), Vec::new())),
                    }
                    continue;
                }
                if events.send(Event::parse(kind, payload)).is_err() && !accepting {
                    break;
                }
            }
        }
    }

    while let Some(reply) = pending.queue.pop_front() {
        let _ = reply.send(Err(Error::Closed));
    }
}
