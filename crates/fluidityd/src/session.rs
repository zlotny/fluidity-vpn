use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fluidity_config::Document;
use fluidity_config::settings::Settings;
use fluidity_ipc::{
    ChoiceOption, ConnectOptions, LogEntry, Prompt, Reply, SessionInfo, SessionState, StaticChallengePrompt,
};
use fluidity_mgmt::{
    Client as Mgmt, DynamicChallenge, Event as MgmtEvent, Need, PasswordRequest, Signal, State, StaticChallenge,
    static_challenge_password,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command as Process};
use tokio::sync::mpsc;

use crate::daemon::{Command, Daemon};
use crate::netconfig::{self, Inputs};
use crate::obfs;
use crate::openvpn;

enum Endpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    #[cfg(windows)]
    Tcp { port: u16, password: String, file: PathBuf },
}

impl Endpoint {
    #[cfg(unix)]
    fn create(dir: &Path) -> Result<Self, String> {
        Ok(Self::Unix(dir.join("mgmt.sock")))
    }

    #[cfg(windows)]
    fn create(dir: &Path) -> Result<Self, String> {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map_err(|e| format!("Cannot reserve a management port: {e}"))?
            .port();
        let password = uuid::Uuid::new_v4().simple().to_string();
        let file = dir.join("mgmt.pw");
        std::fs::write(&file, format!("{password}\n")).map_err(|e| format!("Cannot prepare connection files: {e}"))?;
        Ok(Self::Tcp { port, password, file })
    }

    fn management(&self) -> openvpn::Management<'_> {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => openvpn::Management::Unix(path),
            #[cfg(windows)]
            Self::Tcp { port, file, .. } => openvpn::Management::Tcp {
                port: *port,
                password_file: file,
            },
        }
    }

    async fn connect(&self) -> Option<(Mgmt, mpsc::UnboundedReceiver<MgmtEvent>)> {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => {
                if !path.exists() {
                    return None;
                }
                Mgmt::connect_unix(path).await.ok()
            }
            #[cfg(windows)]
            Self::Tcp { port, password, .. } => {
                Mgmt::connect_tcp(std::net::SocketAddr::from(([127, 0, 0, 1], *port)), password)
                    .await
                    .ok()
            }
        }
    }
}

const RECONNECT_DELAYS: [u64; 6] = [2, 5, 10, 20, 40, 60];

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

pub struct Spec {
    pub generation: u64,
    pub id: String,
    pub document: Document,
    pub files: Vec<(String, Vec<u8>)>,
    pub options: ConnectOptions,
}

enum Pending {
    Credentials {
        realm: String,
        challenge: Option<StaticChallenge>,
    },
    Password {
        realm: String,
    },
    Challenge {
        realm: String,
        challenge: DynamicChallenge,
    },
    NeedOk {
        name: String,
    },
    NeedStr {
        name: String,
    },
    Certificate,
    Sign,
}

enum Outcome {
    Stopped,
    Exited { error: Option<String> },
    AuthCancelled,
}

struct Session {
    daemon: Arc<Daemon>,
    spec: Spec,
    settings: Settings,
    config_options: Vec<Vec<String>>,
    info: SessionInfo,
    commands: mpsc::UnboundedReceiver<Command>,
    pending: Option<Pending>,
    pushed: Vec<Vec<String>>,
    auth_failed: bool,
    net_applied: bool,
    time_machine_held: bool,
    holds: usize,
    release_at: Option<tokio::time::Instant>,
    obfs: Option<obfs::Transport>,
}

pub fn spawn(daemon: Arc<Daemon>, spec: Spec, owner: u32, commands: mpsc::UnboundedReceiver<Command>) {
    let settings = Settings::from_document(&spec.document);
    let config_options = netconfig::document_options(&spec.document);
    let info = SessionInfo {
        id: spec.id.clone(),
        name: settings.name.clone(),
        owner_uid: owner,
        state: SessionState::Connecting,
        ..SessionInfo::default()
    };
    let session = Session {
        daemon,
        spec,
        settings,
        config_options,
        info,
        commands,
        pending: None,
        pushed: Vec::new(),
        auth_failed: false,
        net_applied: false,
        time_machine_held: false,
        holds: 0,
        release_at: None,
        obfs: None,
    };
    tokio::spawn(session.run());
}

fn log_level(flags: &str) -> &'static str {
    if flags.contains('F') {
        "fatal"
    } else if flags.contains('N') {
        "error"
    } else if flags.contains('W') {
        "warning"
    } else if flags.contains('D') {
        "debug"
    } else {
        "info"
    }
}

fn describe(description: &str) -> String {
    match description {
        "SUCCESS" | "SIGUSR1" | "SIGHUP" => "",
        "ERROR" => "Some options failed",
        "tls-error" => "TLS error",
        "socks-error" => "Proxy error",
        "ping-restart" | "ping-timeout" => "Server not responding",
        "connection-reset" => "Connection reset",
        "connection-reset-restart" => "Connection reset",
        "auth-failure" => "Authentication failed",
        "server-pushed-connection-reset" => "Server requested a reconnect",
        "network-change" => "Network changed",
        other => other,
    }
    .to_string()
}

fn session_state(state: State) -> Option<SessionState> {
    Some(match state {
        State::Connecting | State::Wait | State::Resolve | State::TcpConnect => SessionState::Connecting,
        State::Auth | State::AuthPending | State::GetConfig | State::AssignIp | State::AddRoutes => {
            SessionState::Authenticating
        }
        State::Connected => SessionState::Connected,
        State::Reconnecting => SessionState::Reconnecting,
        State::Exiting => SessionState::Disconnecting,
        State::Unknown => return None,
    })
}

impl Session {
    fn run_dir(&self) -> PathBuf {
        let safe: String = self
            .spec
            .id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
            .collect();
        self.daemon.paths.run_dir.join(safe)
    }

    async fn publish(&self) {
        self.daemon.publish(self.spec.generation, &self.info).await;
    }

    async fn log(&self, level: &str, message: impl Into<String>) {
        self.daemon
            .log(
                self.spec.generation,
                &self.spec.id,
                LogEntry {
                    time: now(),
                    level: level.to_string(),
                    message: message.into(),
                },
            )
            .await;
    }

    async fn set_state(&mut self, state: SessionState, detail: &str) {
        self.info.state = state;
        self.info.detail = detail.to_string();
        self.publish().await;
    }

    async fn run(mut self) {
        let mut attempt = 0usize;
        loop {
            let outcome = self.attempt().await;
            self.cleanup().await;
            match outcome {
                Outcome::Stopped => break,
                Outcome::AuthCancelled => {
                    self.info.last_error = Some("Authentication was cancelled".into());
                    break;
                }
                Outcome::Exited { error } => {
                    if let Some(err) = &error {
                        self.info.last_error = Some(err.clone());
                    }
                    let retry = self.spec.options.auto_reconnect && !self.auth_failed;
                    if !retry {
                        self.info.state = if error.is_some() {
                            SessionState::Error
                        } else {
                            SessionState::Disconnected
                        };
                        self.publish().await;
                        break;
                    }
                    let delay = RECONNECT_DELAYS[attempt.min(RECONNECT_DELAYS.len() - 1)];
                    attempt += 1;
                    self.set_state(SessionState::Reconnecting, &format!("Retrying in {delay}s"))
                        .await;
                    if !self.wait_or_stop(Duration::from_secs(delay)).await {
                        break;
                    }
                }
            }
        }
        if self.info.state != SessionState::Error {
            self.info.state = SessionState::Disconnected;
        }
        self.info.connected_since = None;
        self.publish().await;
        let _ = std::fs::remove_dir_all(self.run_dir());
        self.daemon.remove(self.spec.generation, &self.spec.id).await;
    }

    async fn wait_or_stop(&mut self, delay: Duration) -> bool {
        let sleep = tokio::time::sleep(delay);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => return true,
                command = self.commands.recv() => match command {
                    None | Some(Command::Disconnect) => return false,
                    Some(Command::Reply(_)) => {}
                },
            }
        }
    }

    async fn cleanup(&mut self) {
        if self.net_applied {
            let daemon = self.daemon.clone();
            let id = self.spec.id.clone();
            let _ = tokio::task::spawn_blocking(move || daemon.backend.revert(&id)).await;
            self.net_applied = false;
        }
        if self.time_machine_held {
            self.daemon.hold_time_machine(false).await;
            self.time_machine_held = false;
        }
        if self.pending.take().is_some() {
            self.daemon.prompt(self.spec.generation, &self.spec.id, None).await;
        }
        if let Some(transport) = self.obfs.take() {
            transport.stop().await;
        }
        self.info.connected_since = None;
        self.info.bytes_in = 0;
        self.info.bytes_out = 0;
    }

    fn prepare(&self, dir: &Path) -> std::io::Result<()> {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut document = self.spec.document.clone();
        if self.settings.system_identity.is_some() {
            for name in ["cert", "key", "pkcs12", "pkcs11-id", "pkcs11-providers"] {
                document.remove(name);
            }
        }
        std::fs::write(dir.join(fluidity_config::CONFIG_FILE), document.to_string())?;
        for (name, data) in &self.spec.files {
            std::fs::write(dir.join(name), data)?;
        }
        Ok(())
    }

    async fn start_obfuscation(&mut self, dir: &Path) -> Result<(), String> {
        let Some(method) = self.settings.obfuscation else {
            return Ok(());
        };
        let binary = tokio::task::spawn_blocking(obfs::discover)
            .await
            .unwrap_or_default()
            .ok_or("Obfuscation needs lyrebird, which is not installed")?;
        let state = dir.join("pt-state");
        std::fs::create_dir_all(&state).map_err(|e| e.to_string())?;
        let args = obfs::transport_args(method, self.settings.obfuscation_key.as_deref());
        let transport = obfs::start(&binary, method, &state, args).await?;
        let port = transport.port;
        self.obfs = Some(transport);
        self.log("info", format!("Obfuscation ({}) on 127.0.0.1:{port}", method.as_str()))
            .await;
        let document = obfs::rewrite(&self.spec.document, port);
        std::fs::write(dir.join(fluidity_config::CONFIG_FILE), document.to_string())
            .map_err(|e| format!("Cannot prepare connection files: {e}"))
    }

    async fn launch(&self, dir: &Path, endpoint: &Endpoint) -> Result<Child, String> {
        let binaries = tokio::task::spawn_blocking(openvpn::discover).await.unwrap_or_default();
        let requested = self
            .spec
            .options
            .openvpn_version
            .as_deref()
            .or(self.settings.openvpn_version.as_deref());
        let binary = openvpn::select(&binaries, requested).ok_or("OpenVPN is not installed")?;
        let compat = requested
            .filter(|r| openvpn::version_key(r) < openvpn::version_key(&binary.version))
            .filter(|_| openvpn::version_key(&binary.version) >= vec![2, 6])
            .filter(|r| openvpn::version_key(r) >= vec![2, 3])
            .map(|r| format!("{r}.0"));
        let args = openvpn::arguments(
            &openvpn::LaunchSpec {
                management: endpoint.management(),
                gui_version: &self.daemon.gui_version,
                compat_mode: compat.as_deref(),
                query_proxy: self.settings.use_system_proxy,
                external_identity: self.settings.system_identity.is_some(),
            },
            self.spec.document.has("verb"),
        );
        self.log(
            "info",
            format!("Starting OpenVPN {} ({})", binary.version, binary.path.display()),
        )
        .await;
        Process::new(&binary.path)
            .args(&args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Cannot start OpenVPN: {e}"))
    }

    fn forward_output(&self, child: &mut Child) {
        let streams: Vec<Box<dyn tokio::io::AsyncRead + Send + Unpin>> = [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        ]
        .into_iter()
        .flatten()
        .collect();
        for stream in streams {
            let daemon = self.daemon.clone();
            let id = self.spec.id.clone();
            let generation = self.spec.generation;
            tokio::spawn(async move {
                let mut lines = BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    daemon
                        .log(
                            generation,
                            &id,
                            LogEntry {
                                time: now(),
                                level: "output".into(),
                                message: line,
                            },
                        )
                        .await;
                }
            });
        }
    }

    async fn connect_management(
        &self,
        endpoint: &Endpoint,
        child: &mut Child,
    ) -> Result<(Mgmt, mpsc::UnboundedReceiver<MgmtEvent>), String> {
        for _ in 0..150 {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!("OpenVPN exited during startup ({status})"));
            }
            if let Some(pair) = endpoint.connect().await {
                return Ok(pair);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("OpenVPN management interface did not start".into())
    }

    async fn attempt(&mut self) -> Outcome {
        self.auth_failed = false;
        self.holds = 0;
        self.release_at = None;
        self.pushed.clear();
        self.set_state(SessionState::Connecting, "Starting").await;
        let dir = self.run_dir();
        if let Err(err) = self.prepare(&dir) {
            return Outcome::Exited {
                error: Some(format!("Cannot prepare connection files: {err}")),
            };
        }
        if let Err(error) = self.start_obfuscation(&dir).await {
            return Outcome::Exited { error: Some(error) };
        }
        let endpoint = match Endpoint::create(&dir) {
            Ok(endpoint) => endpoint,
            Err(error) => return Outcome::Exited { error: Some(error) },
        };
        let mut child = match self.launch(&dir, &endpoint).await {
            Ok(child) => child,
            Err(error) => return Outcome::Exited { error: Some(error) },
        };
        self.forward_output(&mut child);
        let (mgmt, mut events) = match self.connect_management(&endpoint, &mut child).await {
            Ok(pair) => pair,
            Err(error) => {
                let _ = child.wait().await;
                return Outcome::Exited { error: Some(error) };
            }
        };

        if let Ok(history) = mgmt.command("log on all").await {
            for line in history.lines() {
                self.handle(&mgmt, MgmtEvent::parse("LOG", line)).await;
            }
        }
        if self.settings.system_identity.is_some() {
            let _ = mgmt.version(3).await;
        }
        let _ = mgmt.command("state on").await;
        let _ = mgmt.bytecount(1).await;
        let _ = mgmt.hold_release().await;

        let mut fatal: Option<String> = None;
        let mut stopping = false;
        let mut stop_deadline: Option<tokio::time::Instant> = None;
        let mut auth_cancelled = false;

        loop {
            let deadline = async {
                match stop_deadline {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            let release_at = self.release_at;
            let release = async {
                match release_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                event = events.recv() => match event {
                    Some(MgmtEvent::Fatal(msg)) => {
                        self.log("fatal", msg.clone()).await;
                        fatal = Some(msg);
                    }
                    Some(event) => {
                        if self.handle(&mgmt, event).await {
                            auth_cancelled = true;
                        }
                    }
                    None => {
                        let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
                        let _ = child.kill().await;
                        break;
                    }
                },
                command = self.commands.recv(), if !stopping => match command {
                    None | Some(Command::Disconnect) => {
                        stopping = true;
                        self.set_state(SessionState::Disconnecting, "Disconnecting").await;
                        let _ = mgmt.signal(Signal::Term).await;
                        stop_deadline = Some(tokio::time::Instant::now() + Duration::from_secs(10));
                    }
                    Some(Command::Reply(reply)) => {
                        if self.reply(&mgmt, reply).await {
                            auth_cancelled = true;
                            stopping = true;
                            let _ = mgmt.signal(Signal::Term).await;
                            stop_deadline = Some(tokio::time::Instant::now() + Duration::from_secs(10));
                        }
                    }
                },
                status = child.wait() => {
                    if let Ok(status) = status && !status.success() && fatal.is_none() && !stopping {
                        fatal = Some(format!("OpenVPN exited unexpectedly ({status})"));
                    }
                    break;
                }
                _ = deadline => {
                    let _ = child.kill().await;
                    break;
                }
                _ = release, if !stopping => {
                    self.release_at = None;
                    let _ = mgmt.hold_release().await;
                }
            }
        }

        if auth_cancelled {
            Outcome::AuthCancelled
        } else if stopping {
            Outcome::Stopped
        } else {
            let error = fatal.or_else(|| self.auth_failed.then(|| "Authentication failed".to_string()));
            Outcome::Exited { error }
        }
    }

    async fn handle(&mut self, mgmt: &Mgmt, event: MgmtEvent) -> bool {
        match event {
            MgmtEvent::Log(line) => {
                if let Some(opts) = netconfig::parse_push_reply(&line.message) {
                    self.pushed = opts;
                }
                self.log(log_level(&line.flags), line.message).await;
            }
            MgmtEvent::State(change) => {
                if let Some(state) = session_state(change.state) {
                    self.info.state = state;
                    self.info.detail = describe(&change.description);
                    if state == SessionState::Connected {
                        self.info.local_ipv4 = change.local_ip.clone();
                        self.info.local_ipv6 = change.local_ipv6.clone();
                        self.info.remote_address = change.remote_ip.clone();
                        self.info.remote_port = change.remote_port.clone();
                        self.info.connected_since = Some(change.time.max(1));
                        self.info.last_error = None;
                        self.holds = 1;
                        if self.pending.take().is_some() {
                            self.daemon.prompt(self.spec.generation, &self.spec.id, None).await;
                        }
                        if self.spec.options.disable_time_machine && !self.time_machine_held {
                            self.daemon.hold_time_machine(true).await;
                            self.time_machine_held = true;
                        }
                    }
                    if state == SessionState::Reconnecting {
                        self.info.connected_since = None;
                    }
                    self.publish().await;
                }
            }
            MgmtEvent::ByteCount { bytes_in, bytes_out } => {
                self.info.bytes_in = bytes_in;
                self.info.bytes_out = bytes_out;
                self.publish().await;
            }
            MgmtEvent::Hold(_) => {
                self.holds += 1;
                if self.holds <= 1 {
                    let _ = mgmt.hold_release().await;
                } else {
                    let delay = RECONNECT_DELAYS[(self.holds - 2).min(RECONNECT_DELAYS.len() - 1)];
                    self.release_at = Some(tokio::time::Instant::now() + Duration::from_secs(delay));
                    self.set_state(SessionState::Reconnecting, &format!("Retrying in {delay}s"))
                        .await;
                }
            }
            MgmtEvent::Password(request) => return self.password(request).await,
            MgmtEvent::NeedOk { name, message } => {
                self.pending = Some(Pending::NeedOk { name: name.clone() });
                self.daemon
                    .prompt(
                        self.spec.generation,
                        &self.spec.id,
                        Some(Prompt::NeedOk { name, message }),
                    )
                    .await;
            }
            MgmtEvent::NeedStr { name, message } if name == "pkcs11-id-request" => {
                let options: Vec<ChoiceOption> = mgmt
                    .pkcs11_ids()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| ChoiceOption {
                        label: id.label(),
                        value: id.id,
                    })
                    .collect();
                if options.is_empty() {
                    self.log("error", "No PKCS#11 tokens found. Connect the token and try again.")
                        .await;
                }
                self.pending = Some(Pending::NeedStr { name: name.clone() });
                self.daemon
                    .prompt(
                        self.spec.generation,
                        &self.spec.id,
                        Some(Prompt::Choice { name, message, options }),
                    )
                    .await;
            }
            MgmtEvent::NeedStr { name, message } => {
                self.pending = Some(Pending::NeedStr { name: name.clone() });
                self.daemon
                    .prompt(
                        self.spec.generation,
                        &self.spec.id,
                        Some(Prompt::NeedStr { name, message }),
                    )
                    .await;
            }
            MgmtEvent::InfoMsg(ref msg) => {
                if let Some(url) = event.web_auth_url() {
                    self.daemon
                        .prompt(
                            self.spec.generation,
                            &self.spec.id,
                            Some(Prompt::WebAuth { url: url.to_string() }),
                        )
                        .await;
                } else {
                    self.log("info", msg.clone()).await;
                }
            }
            MgmtEvent::NeedCertificate(_) => {
                let Some(identity) = self.settings.system_identity.clone() else {
                    return false;
                };
                self.pending = Some(Pending::Certificate);
                self.daemon
                    .prompt(
                        self.spec.generation,
                        &self.spec.id,
                        Some(Prompt::Certificate { identity }),
                    )
                    .await;
            }
            MgmtEvent::PkSign { data, algorithm } => {
                let Some(identity) = self.settings.system_identity.clone() else {
                    return false;
                };
                self.pending = Some(Pending::Sign);
                let prompt = Prompt::Sign {
                    identity,
                    data,
                    algorithm: algorithm.unwrap_or_default(),
                };
                self.daemon
                    .prompt(self.spec.generation, &self.spec.id, Some(prompt))
                    .await;
            }
            MgmtEvent::UpDown { direction, env } => self.updown(&direction, &env).await,
            MgmtEvent::Remote { .. } => {
                let _ = mgmt.remote(fluidity_mgmt::RemoteAction::Accept).await;
            }
            MgmtEvent::Proxy { proto, host, .. } => {
                let proxies = tokio::task::spawn_blocking(crate::proxy::SystemProxies::current)
                    .await
                    .unwrap_or_default();
                let action = proxies.choose(&host, &proto);
                if let fluidity_mgmt::ProxyAction::Http { host: ref p, port, .. }
                | fluidity_mgmt::ProxyAction::Socks { host: ref p, port } = action
                {
                    self.log("info", format!("Using system proxy {p}:{port}")).await;
                }
                let _ = mgmt.proxy(action).await;
            }
            _ => {}
        }
        false
    }

    async fn password(&mut self, request: PasswordRequest) -> bool {
        match request {
            PasswordRequest::Need {
                realm,
                need,
                static_challenge,
            } => {
                let prompt = match need {
                    Need::UsernamePassword => Prompt::Credentials {
                        realm: realm.clone(),
                        static_challenge: static_challenge.as_ref().map(|sc| StaticChallengePrompt {
                            text: sc.text.clone(),
                            echo: sc.echo,
                        }),
                        retry: self.auth_failed,
                    },
                    Need::Password => Prompt::Password { realm: realm.clone() },
                };
                self.pending = Some(match need {
                    Need::UsernamePassword => Pending::Credentials {
                        realm,
                        challenge: static_challenge,
                    },
                    Need::Password => Pending::Password { realm },
                });
                self.daemon
                    .prompt(self.spec.generation, &self.spec.id, Some(prompt))
                    .await;
            }
            PasswordRequest::VerificationFailed { realm, challenge } => match challenge {
                Some(challenge) => {
                    let prompt = Prompt::Challenge {
                        realm: realm.clone(),
                        text: challenge.text.clone(),
                        echo: challenge.echo,
                    };
                    self.pending = Some(Pending::Challenge { realm, challenge });
                    self.daemon
                        .prompt(self.spec.generation, &self.spec.id, Some(prompt))
                        .await;
                }
                None => {
                    self.auth_failed = true;
                    self.info.last_error = Some("Authentication failed".into());
                    self.log("error", format!("Authentication failed ({realm})")).await;
                    self.publish().await;
                }
            },
            PasswordRequest::AuthToken(_) => {}
        }
        false
    }

    async fn reply(&mut self, mgmt: &Mgmt, reply: Reply) -> bool {
        let Some(pending) = self.pending.take() else {
            return false;
        };
        self.daemon.prompt(self.spec.generation, &self.spec.id, None).await;
        let result = match (pending, reply) {
            (_, Reply::Cancel) => return true,
            (
                Pending::Credentials { realm, challenge },
                Reply::Credentials {
                    username,
                    password,
                    challenge_response,
                    ..
                },
            ) => {
                let password = match (challenge, challenge_response) {
                    (Some(sc), Some(response)) if sc.concat => format!("{password}{response}"),
                    (Some(_), Some(response)) => static_challenge_password(&password, &response),
                    _ => password,
                };
                mgmt.credentials(&realm, &username, &password).await
            }
            (Pending::Password { realm }, Reply::Password { password, .. })
            | (Pending::Password { realm }, Reply::Credentials { password, .. }) => {
                mgmt.password(&realm, &password).await.map(|_| ())
            }
            (Pending::Challenge { realm, challenge }, Reply::Password { password, .. })
            | (Pending::Challenge { realm, challenge }, Reply::Credentials { password, .. }) => {
                mgmt.challenge_response(&realm, &challenge, &password).await
            }
            (Pending::NeedOk { name }, Reply::NeedOk { ok, .. }) => mgmt.need_ok(&name, ok).await.map(|_| ()),
            (Pending::NeedStr { name }, Reply::NeedStr { value, .. }) => mgmt.need_str(&name, &value).await.map(|_| ()),
            (Pending::Certificate, Reply::Certificate { pem }) => mgmt.certificate(&pem).await.map(|_| ()),
            (Pending::Sign, Reply::Signature { signature }) => mgmt.pk_signature(&signature).await.map(|_| ()),
            (pending, _) => {
                self.pending = Some(pending);
                return false;
            }
        };
        if let Err(err) = result {
            self.log("error", format!("Cannot send reply to OpenVPN: {err}")).await;
        }
        false
    }

    async fn updown(&mut self, direction: &str, env: &[(String, String)]) {
        let get = |key: &str| {
            env.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
                .unwrap_or_default()
        };
        if direction != "UP" {
            if self.net_applied {
                let daemon = self.daemon.clone();
                let id = self.spec.id.clone();
                let _ = tokio::task::spawn_blocking(move || daemon.backend.revert(&id)).await;
                self.net_applied = false;
            }
            return;
        }
        self.info.interface = get("dev").to_string();
        let config = netconfig::resolve(&Inputs {
            settings: &self.settings,
            config: &self.config_options,
            pushed: &self.pushed,
            interface: get("dev"),
            tunnel_has_ipv6: !get("ifconfig_ipv6_local").is_empty(),
            block_ipv6: self.spec.options.block_ipv6,
        });
        if netconfig::split_dns_warning(&config) {
            self.log("warning", "Split DNS is in use but no DNS domains are set")
                .await;
        }
        if config.is_empty() {
            return;
        }
        let daemon = self.daemon.clone();
        let id = self.spec.id.clone();
        let applied = config.clone();
        let result = tokio::task::spawn_blocking(move || daemon.backend.apply(&id, &applied))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        self.net_applied = true;
        match result {
            Ok(()) => {
                if let Some(mode) = config.dns {
                    self.log(
                        "info",
                        format!(
                            "DNS ({mode:?}): servers {}, domains {}",
                            config.servers.join(", "),
                            config.match_domains.join(", ")
                        ),
                    )
                    .await;
                }
            }
            Err(err) => self.log("error", format!("Network settings: {err}")).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_reasons() {
        assert_eq!(describe("SUCCESS"), "");
        assert_eq!(describe("tls-error"), "TLS error");
        assert_eq!(describe("custom"), "custom");
    }
}
