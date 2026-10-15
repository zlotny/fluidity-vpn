use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use fluidity_core::{CredentialStore, Credentials, Node, Preferences, Store, wire_bundle};
use fluidity_ipc::{Client, ConnectOptions, Event, Prompt, Reply, Request, SessionInfo, SessionState, SystemSettings};

#[derive(Parser)]
#[command(name = "fluidity", version, about = "Control Fluidity VPN connections")]
struct Cli {
    #[arg(long, global = true, help = "Print machine-readable JSON")]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "List saved connections and their state")]
    List,
    #[command(about = "Show active connections")]
    Status,
    #[command(about = "Connect a saved connection")]
    Connect {
        name: String,
        #[arg(long, help = "Return once the request is sent")]
        no_wait: bool,
        #[arg(long, help = "Save entered credentials")]
        save: bool,
    },
    #[command(about = "Disconnect a connection")]
    Disconnect {
        name: Option<String>,
        #[arg(long)]
        all: bool,
    },
    #[command(about = "Allow internet traffic that the kill switch blocks")]
    Unblock { name: Option<String> },
    #[command(about = "Connect every connection")]
    ConnectAll,
    #[command(about = "Disconnect every connection")]
    DisconnectAll,
    #[command(about = "Import .ovpn, .conf, .fvpn, .fvpnz, .tblk or zip files")]
    Import {
        paths: Vec<PathBuf>,
        #[arg(long, help = "Replace connections with the same name")]
        replace: bool,
    },
    #[command(about = "Export a connection as .fvpnz or .fvpn")]
    Export { name: String, path: PathBuf },
    #[command(about = "Delete a saved connection")]
    Remove { name: String },
    #[command(about = "Duplicate a saved connection")]
    Duplicate { name: String },
    #[command(about = "Rename a saved connection")]
    Rename { name: String, new_name: String },
    #[command(about = "Show the connection log")]
    Log {
        name: String,
        #[arg(short, long)]
        follow: bool,
    },
    #[command(about = "Delete saved credentials")]
    ForgetCredentials { name: Option<String> },
    #[command(about = "List available OpenVPN versions")]
    Versions,
    #[command(about = "List certificate identities in the system keychain")]
    Identities,
    #[command(about = "Apply a preconfigured deployment")]
    Deploy {
        #[arg(long, help = "Deployment folder (default: the system deployment folder)")]
        dir: Option<PathBuf>,
        #[arg(long, value_parser = ["append", "replace", "overwrite"], help = "How to treat existing connections")]
        mode: Option<String>,
        #[arg(long, help = "Apply even if this version was applied before")]
        force: bool,
    },
    #[command(about = "Show or change service settings (changes need root)")]
    SystemSettings {
        #[arg(long)]
        allow_unsafe_commands: Option<bool>,
    },
}

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("fluidity: {message}");
    ExitCode::FAILURE
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    match run(cli).await {
        Ok(code) => code,
        Err(err) => fail(err),
    }
}

async fn daemon() -> Result<(Client, tokio::sync::mpsc::UnboundedReceiver<Event>), String> {
    Client::connect(&fluidity_ipc::socket_path())
        .await
        .map_err(|e| e.to_string())
}

fn state_label(state: SessionState) -> &'static str {
    match state {
        SessionState::Disconnected => "Disconnected",
        SessionState::Connecting => "Connecting",
        SessionState::Authenticating => "Authenticating",
        SessionState::Connected => "Connected",
        SessionState::Reconnecting => "Reconnecting",
        SessionState::Disconnecting => "Disconnecting",
        SessionState::Error => "Error",
    }
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

async fn sessions() -> Vec<SessionInfo> {
    match daemon().await {
        Ok((client, _)) => client.sessions().await.unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn credential_scope(store: &Store, conn: &fluidity_core::Connection) -> String {
    conn.folder
        .as_deref()
        .and_then(|f| store.folder(f))
        .filter(|f| f.shared_credentials)
        .map(|f| f.id)
        .unwrap_or_else(|| conn.id.clone())
}

async fn run(cli: Cli) -> Result<ExitCode, String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    match cli.command {
        Commands::List => {
            let active = sessions().await;
            let connections = store.connections();
            if cli.json {
                let rows: Vec<_> = connections
                    .iter()
                    .map(|c| {
                        let s = active.iter().find(|s| s.id == c.id);
                        serde_json::json!({
                            "id": c.id,
                            "name": c.name,
                            "folder": c.folder,
                            "state": s.map(|s| state_label(s.state)).unwrap_or("Disconnected"),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows).unwrap_or_default());
                return Ok(ExitCode::SUCCESS);
            }
            fn print(store: &Store, nodes: &[Node], active: &[SessionInfo], indent: usize) {
                for node in nodes {
                    match node {
                        Node::Folder(f) => {
                            println!("{:indent$}{}/", "", f.name);
                            print(store, &f.children, active, indent + 2);
                        }
                        Node::Connection(id) => {
                            let Ok(c) = store.find(id) else { continue };
                            let state = active
                                .iter()
                                .find(|s| &s.id == id)
                                .map(|s| state_label(s.state))
                                .unwrap_or("Disconnected");
                            println!("{:indent$}{:<32} {state}", "", c.name);
                        }
                    }
                }
            }
            print(&store, &store.tree(), &active, 0);
        }
        Commands::Status => {
            let (client, _) = daemon().await?;
            let list = client.sessions().await.map_err(|e| e.to_string())?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&list).unwrap_or_default());
                return Ok(ExitCode::SUCCESS);
            }
            if list.is_empty() {
                println!("No active connections");
            }
            for s in list {
                let name = store.find(&s.id).map(|c| c.name).unwrap_or_else(|_| s.name.clone());
                println!("{name}: {}", state_label(s.state));
                if s.state == SessionState::Connected {
                    println!(
                        "  Interface: {}  Address: {} {}",
                        s.interface, s.local_ipv4, s.local_ipv6
                    );
                    println!("  Server: {}:{}", s.remote_address, s.remote_port);
                    if s.kill_switch {
                        println!("  Kill switch: on");
                    }
                    println!(
                        "  Traffic: in {}  out {}",
                        human_bytes(s.bytes_in),
                        human_bytes(s.bytes_out)
                    );
                }
                if let Some(err) = s.last_error {
                    println!("  Error: {err}");
                }
            }
            if let Ok(fluidity_ipc::Payload::Blocked(ids)) = client.request(Request::Blocked).await {
                for id in ids {
                    let name = store.find(&id).map(|c| c.name).unwrap_or(id);
                    println!(
                        "Kill switch: internet blocked since {name} dropped. Run \"fluidity unblock\" to allow it."
                    );
                }
            }
        }
        Commands::Connect { name, no_wait, save } => {
            let conn = store.find(&name).map_err(|e| e.to_string())?;
            return connect(&store, conn, !no_wait, save).await;
        }
        Commands::ConnectAll => {
            let mut code = ExitCode::SUCCESS;
            for conn in store.connections() {
                if connect(&store, conn, false, false).await? != ExitCode::SUCCESS {
                    code = ExitCode::FAILURE;
                }
            }
            return Ok(code);
        }
        Commands::Disconnect { name, all } => {
            let (client, _) = daemon().await?;
            let request = match (name, all) {
                (_, true) => Request::DisconnectAll,
                (Some(name), false) => {
                    let conn = store.find(&name).map_err(|e| e.to_string())?;
                    Request::Disconnect { id: conn.id }
                }
                (None, false) => return Err("name a connection or pass --all".into()),
            };
            client.request(request).await.map_err(|e| e.to_string())?;
        }
        Commands::DisconnectAll => {
            let (client, _) = daemon().await?;
            client
                .request(Request::DisconnectAll)
                .await
                .map_err(|e| e.to_string())?;
        }
        Commands::Import { paths, replace } => {
            let mut code = ExitCode::SUCCESS;
            for path in paths {
                match store.import_replacing(&path, replace) {
                    Ok(c) => println!("Imported \"{}\"", c.name),
                    Err(e) => {
                        eprintln!("fluidity: {}: {e}", path.display());
                        code = ExitCode::FAILURE;
                    }
                }
            }
            return Ok(code);
        }
        Commands::Export { name, path } => {
            let conn = store.find(&name).map_err(|e| e.to_string())?;
            store.export(&conn.id, &path).map_err(|e| e.to_string())?;
        }
        Commands::Remove { name } => {
            let conn = store.find(&name).map_err(|e| e.to_string())?;
            store.remove(&conn.id).map_err(|e| e.to_string())?;
            CredentialStore::clear(&conn.id);
        }
        Commands::Duplicate { name } => {
            let conn = store.find(&name).map_err(|e| e.to_string())?;
            let copy = store.duplicate(&conn.id).map_err(|e| e.to_string())?;
            println!("Created \"{}\"", copy.name);
        }
        Commands::Rename { name, new_name } => {
            let conn = store.find(&name).map_err(|e| e.to_string())?;
            store.rename(&conn.id, &new_name).map_err(|e| e.to_string())?;
        }
        Commands::Log { name, follow } => {
            let conn = store.find(&name).map_err(|e| e.to_string())?;
            let (client, mut events) = daemon().await?;
            if follow {
                client.subscribe().await.map_err(|e| e.to_string())?;
            }
            for entry in client.log(&conn.id).await.map_err(|e| e.to_string())? {
                println!("{} {}", entry.time, entry.message);
            }
            if follow {
                while let Some(event) = events.recv().await {
                    match event {
                        Event::Log { id, entry } if id == conn.id => println!("{} {}", entry.time, entry.message),
                        Event::Removed { id } if id == conn.id => break,
                        _ => {}
                    }
                }
            }
        }
        Commands::ForgetCredentials { name } => match name {
            Some(name) => {
                let conn = store.find(&name).map_err(|e| e.to_string())?;
                CredentialStore::clear(&credential_scope(&store, &conn));
            }
            None => {
                for conn in store.connections() {
                    CredentialStore::clear(&credential_scope(&store, &conn));
                }
            }
        },
        Commands::Deploy { dir, mode, force } => {
            use fluidity_core::deploy::{ConnectionMode, Deployment};
            let store = Store::open_default().map_err(|e| e.to_string())?;
            let dirs = dir.map(|d| vec![d]).unwrap_or_else(fluidity_core::deploy::system_dirs);
            let deployment = if force {
                dirs.iter().find_map(|d| Deployment::open(d))
            } else {
                Deployment::pending(&store, &dirs)
            };
            let Some(deployment) = deployment else {
                println!("No pending deployment");
                return Ok(ExitCode::SUCCESS);
            };
            let mode = match mode.as_deref() {
                Some("append") => ConnectionMode::Append,
                Some("overwrite") => ConnectionMode::Overwrite,
                Some(_) => ConnectionMode::Replace,
                None if deployment.manifest.connections == ConnectionMode::Prompt => ConnectionMode::Replace,
                None => deployment.manifest.connections,
            };
            let report = deployment.apply(&store, mode).map_err(|e| e.to_string())?;
            for (label, names) in [
                ("Added", &report.added),
                ("Updated", &report.updated),
                ("Removed", &report.removed),
                ("Kept", &report.skipped),
            ] {
                if !names.is_empty() {
                    println!("{label}: {}", names.join(", "));
                }
            }
            for error in &report.errors {
                eprintln!("fluidity: {error}");
            }
        }
        Commands::Identities => {
            for name in fluidity_core::identity::names() {
                println!("{name}");
            }
        }
        Commands::Unblock { name } => {
            let id = match name {
                Some(name) => Some(store.find(&name).map_err(|e| e.to_string())?.id),
                None => None,
            };
            let (client, _) = daemon().await?;
            client
                .request(Request::Unblock { id })
                .await
                .map_err(|e| e.to_string())?;
        }
        Commands::Versions => {
            let (client, _) = daemon().await?;
            if let fluidity_ipc::Payload::Versions(v) =
                client.request(Request::Versions).await.map_err(|e| e.to_string())?
            {
                for version in v {
                    println!("{version}");
                }
            }
        }
        Commands::SystemSettings { allow_unsafe_commands } => {
            let (client, _) = daemon().await?;
            match allow_unsafe_commands {
                Some(allow) => {
                    client
                        .request(Request::SetSystemSettings {
                            settings: SystemSettings {
                                allow_unsafe_commands: allow,
                            },
                        })
                        .await
                        .map_err(|e| e.to_string())?;
                }
                None => {
                    if let fluidity_ipc::Payload::Settings(s) = client
                        .request(Request::SystemSettings)
                        .await
                        .map_err(|e| e.to_string())?
                    {
                        println!("allow_unsafe_commands: {}", s.allow_unsafe_commands);
                    }
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn ask(question: &str, secret: bool) -> Result<String, String> {
    if secret && std::io::stdin().is_terminal() {
        return rpassword::prompt_password(question).map_err(|e| e.to_string());
    }
    eprint!("{question}");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

fn open_url(url: &str) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(url).spawn();
}

async fn connect(store: &Store, conn: fluidity_core::Connection, wait: bool, save: bool) -> Result<ExitCode, String> {
    let bundle = store.load(&conn.id).map_err(|e| e.to_string())?;
    let prefs = Preferences::load(store.root());
    let (client, mut events) = daemon().await?;
    client.subscribe().await.map_err(|e| e.to_string())?;
    client
        .request(Request::Connect {
            id: conn.id.clone(),
            bundle: wire_bundle(&bundle),
            options: ConnectOptions {
                openvpn_version: prefs.openvpn_version.clone(),
                auto_reconnect: conn.settings.auto_reconnect && !wait,
                block_ipv6: prefs.block_ipv6,
                kill_switch: prefs.kill_switch,
                kill_switch_allow_lan: prefs.kill_switch_allow_lan,
                disable_time_machine: prefs.disable_time_machine,
            },
        })
        .await
        .map_err(|e| e.to_string())?;
    if !wait {
        return Ok(ExitCode::SUCCESS);
    }

    let scope = credential_scope(store, &conn);
    let mut last_state = None;
    while let Some(event) = events.recv().await {
        match event {
            Event::Session { session } if session.id == conn.id => {
                if last_state != Some(session.state) {
                    eprintln!("{}: {}", conn.name, state_label(session.state));
                    last_state = Some(session.state);
                }
                match session.state {
                    SessionState::Connected => {
                        println!(
                            "Connected: {} {} via {}",
                            session.local_ipv4, session.local_ipv6, session.interface
                        );
                        return Ok(ExitCode::SUCCESS);
                    }
                    SessionState::Error => {
                        return Err(session.last_error.unwrap_or_else(|| "connection failed".into()));
                    }
                    _ => {}
                }
            }
            Event::Removed { id } if id == conn.id => return Err("connection ended".into()),
            Event::Prompt { id, prompt } if id == conn.id => {
                if let Some(reply) = answer(&conn, &scope, prompt, save)? {
                    client.reply(&conn.id, reply).await.map_err(|e| e.to_string())?;
                }
            }
            _ => {}
        }
    }
    Err("lost contact with the Fluidity service".into())
}

fn answer(conn: &fluidity_core::Connection, scope: &str, prompt: Prompt, save: bool) -> Result<Option<Reply>, String> {
    Ok(Some(match prompt {
        Prompt::Credentials {
            realm,
            static_challenge,
            retry,
        } => {
            let saved = if retry || realm != "Auth" {
                Credentials::default()
            } else {
                CredentialStore::load(scope)
            };
            if retry {
                eprintln!("Authentication failed. Enter your credentials again.");
            }
            let label = if realm == "Auth" {
                conn.name.clone()
            } else {
                format!("{} {realm}", conn.name)
            };
            let username = match saved.username {
                Some(u) => u,
                None => ask(&format!("{label} username: "), false)?,
            };
            let mut password = match saved.password {
                Some(p) => p,
                None => ask(&format!("{label} password: "), true)?,
            };
            if save && realm == "Auth" {
                let to_save = Credentials {
                    username: Some(username.clone()),
                    password: Some(password.clone()),
                };
                if let Err(e) = CredentialStore::save(scope, &to_save) {
                    eprintln!("fluidity: cannot save credentials: {e}");
                }
            }
            if let Some(extra) = conn
                .settings
                .static_challenge_password
                .as_ref()
                .filter(|_| realm == "Auth")
            {
                let code = ask(&format!("{}: ", extra.text), false)?;
                password = format!("{password}{}{code}", extra.delimiter);
            }
            let challenge_response = match static_challenge {
                Some(sc) => Some(ask(&format!("{}: ", sc.text), !sc.echo)?),
                None => None,
            };
            Reply::Credentials {
                realm,
                username,
                password,
                challenge_response,
            }
        }
        Prompt::Password { realm } => Reply::Password {
            password: ask(&format!("{} {realm} password: ", conn.name), true)?,
            realm,
        },
        Prompt::Challenge { realm, text, echo } => Reply::Password {
            password: ask(&format!("{text}: "), !echo)?,
            realm,
        },
        Prompt::NeedOk { name, message } => {
            let answer = ask(&format!("{message} [y/N]: "), false)?;
            Reply::NeedOk {
                name,
                ok: answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"),
            }
        }
        Prompt::NeedStr { name, message } => Reply::NeedStr {
            value: ask(&format!("{message}: "), false)?,
            name,
        },
        Prompt::WebAuth { url } => {
            eprintln!("Complete sign-in in your browser: {url}");
            open_url(&url);
            return Ok(None);
        }
        Prompt::Choice { name, message, options } => {
            if options.is_empty() {
                return Err(format!("{message}: nothing to choose from"));
            }
            eprintln!("{message}");
            for (i, option) in options.iter().enumerate() {
                eprintln!("  {}. {}", i + 1, option.label);
            }
            let answer = ask(&format!("Choose 1-{}: ", options.len()), false)?;
            let index = answer
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=options.len()).contains(n))
                .ok_or("invalid choice")?;
            Reply::NeedStr {
                name,
                value: options[index - 1].value.clone(),
            }
        }
        Prompt::Certificate { identity } => Reply::Certificate {
            pem: fluidity_core::identity::certificate(&identity)?,
        },
        Prompt::Sign {
            identity,
            data,
            algorithm,
        } => Reply::Signature {
            signature: fluidity_core::identity::sign(&identity, &data, &algorithm)?,
        },
    }))
}
