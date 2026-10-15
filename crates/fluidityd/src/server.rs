use std::collections::VecDeque;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fluidity_config::{Document, safety};
use fluidity_ipc::{
    ClientMessage, Event, PROTOCOL_VERSION, Payload, Request, ServerMessage, SessionInfo, SessionState, read_message,
    write_message,
};
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::sync::{broadcast, mpsc};

use crate::daemon::{Command, Daemon, Entry};
use crate::openvpn;
use crate::session::{self, Spec};

pub const ROOT: u32 = 0;

pub async fn serve<S>(daemon: Arc<Daemon>, stream: S, uid: u32)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (read, mut write) = tokio::io::split(stream);
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ServerMessage>();
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            if write_message(&mut write, &message).await.is_err() {
                break;
            }
        }
    });

    let mut reader = BufReader::new(read);
    let mut buf = String::new();
    let mut forwarder: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        let message = match read_message::<_, ClientMessage>(&mut reader, &mut buf).await {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(err) => {
                tracing::debug!("client {uid}: {err}");
                break;
            }
        };
        let seq = message.seq;
        if matches!(message.request, Request::Subscribe) && forwarder.is_none() {
            forwarder = Some(spawn_forwarder(daemon.events.subscribe(), out_tx.clone(), uid));
        }
        let reply = match handle(&daemon, uid, message.request).await {
            Ok(ok) => ServerMessage::Ok { seq, ok },
            Err(error) => ServerMessage::Err { seq, error },
        };
        if out_tx.send(reply).is_err() {
            break;
        }
    }

    if let Some(forwarder) = forwarder {
        forwarder.abort();
    }
    drop(out_tx);
    let _ = writer.await;
}

fn spawn_forwarder(
    mut events: broadcast::Receiver<(u32, Event)>,
    out: mpsc::UnboundedSender<ServerMessage>,
    uid: u32,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok((owner, event)) => {
                    if (uid == ROOT || owner == uid) && out.send(ServerMessage::Event(event)).is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

fn can_access(uid: u32, owner: u32) -> bool {
    uid == ROOT || uid == owner
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

fn valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != fluidity_config::CONFIG_FILE
        && name != "mgmt.sock"
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
}

async fn handle(daemon: &Arc<Daemon>, uid: u32, request: Request) -> Result<Payload, String> {
    match request {
        Request::Hello { .. } => Ok(Payload::Hello {
            version: PROTOCOL_VERSION,
            daemon: env!("CARGO_PKG_VERSION").to_string(),
        }),
        Request::Subscribe => Ok(Payload::Empty {}),
        Request::List => {
            let sessions = daemon.sessions.lock().await;
            let mut list: Vec<SessionInfo> = sessions
                .values()
                .filter(|e| can_access(uid, e.owner))
                .map(|e| e.info.clone())
                .collect();
            list.sort_by(|a, b| a.id.cmp(&b.id));
            let prompts: Vec<Event> = sessions
                .iter()
                .filter(|(_, e)| can_access(uid, e.owner))
                .filter_map(|(id, e)| e.prompt.clone().map(|prompt| Event::Prompt { id: id.clone(), prompt }))
                .collect();
            for prompt in prompts {
                let _ = daemon.events.send((uid, prompt));
            }
            Ok(Payload::Sessions(list))
        }
        Request::Connect { id, bundle, options } => {
            if !valid_id(&id) {
                return Err("invalid connection id".into());
            }
            let document = Document::parse(&bundle.config).map_err(|e| e.to_string())?;
            let violations = safety::check(&document);
            let allow_unsafe = daemon.settings.read().await.allow_unsafe_commands;
            if let Some(v) = violations.iter().find(|v| match v {
                safety::Violation::Reserved(_) => true,
                safety::Violation::Unsafe(_) => !allow_unsafe,
            }) {
                return Err(format!(
                    "The OpenVPN command \"{}\" is not allowed. Remove it from the connection's advanced commands.",
                    v.directive()
                ));
            }
            if !allow_unsafe {
                let names: Vec<&str> = bundle.files.keys().map(String::as_str).collect();
                if let Some(path) = safety::foreign_files(&document, &names).first() {
                    return Err(format!(
                        "The file \"{path}\" is not part of the connection. Import the connection again with its files."
                    ));
                }
                for args in document.directives("pkcs11-providers") {
                    for provider in args {
                        let path = std::path::Path::new(provider);
                        if !path.is_absolute() || !crate::openvpn::trusted(path) {
                            return Err(format!(
                                "The PKCS#11 provider \"{provider}\" must be a system library owned by root."
                            ));
                        }
                    }
                }
            }
            let mut files = Vec::new();
            for (name, data) in bundle.files {
                if !valid_file_name(&name) {
                    return Err(format!("invalid file name: {name}"));
                }
                files.push((name, STANDARD.decode(data).map_err(|e| e.to_string())?));
            }
            let (tx, rx) = mpsc::unbounded_channel();
            let generation = daemon.next_generation();
            {
                let mut sessions = daemon.sessions.lock().await;
                if let Some(existing) = sessions.get(&id) {
                    if !can_access(uid, existing.owner) {
                        return Err("this connection belongs to another user".into());
                    }
                    if existing.info.state.is_active() {
                        return Err("already connected".into());
                    }
                }
                sessions.insert(
                    id.clone(),
                    Entry {
                        generation,
                        owner: uid,
                        commands: tx,
                        info: SessionInfo {
                            id: id.clone(),
                            owner_uid: uid,
                            state: SessionState::Connecting,
                            ..SessionInfo::default()
                        },
                        log: VecDeque::new(),
                        prompt: None,
                    },
                );
            }
            if options.kill_switch {
                daemon.adopt_kill_switch(&id).await;
            } else {
                daemon.release_kill_switch(uid, Some(&id)).await;
            }
            session::spawn(
                daemon.clone(),
                Spec {
                    generation,
                    id,
                    document,
                    files,
                    options,
                },
                uid,
                rx,
            );
            Ok(Payload::Empty {})
        }
        Request::Disconnect { id } => {
            let sessions = daemon.sessions.lock().await;
            let entry = sessions.get(&id).ok_or("not connected")?;
            if !can_access(uid, entry.owner) {
                return Err("this connection belongs to another user".into());
            }
            let _ = entry.commands.send(Command::Disconnect);
            Ok(Payload::Empty {})
        }
        Request::DisconnectAll => {
            let sessions = daemon.sessions.lock().await;
            for entry in sessions.values().filter(|e| can_access(uid, e.owner)) {
                let _ = entry.commands.send(Command::Disconnect);
            }
            Ok(Payload::Empty {})
        }
        Request::Reply { id, reply } => {
            let sessions = daemon.sessions.lock().await;
            let entry = sessions.get(&id).ok_or("not connected")?;
            if !can_access(uid, entry.owner) {
                return Err("this connection belongs to another user".into());
            }
            let _ = entry.commands.send(Command::Reply(reply));
            Ok(Payload::Empty {})
        }
        Request::Log { id } => {
            let sessions = daemon.sessions.lock().await;
            let entry = sessions.get(&id).ok_or("not connected")?;
            if !can_access(uid, entry.owner) {
                return Err("this connection belongs to another user".into());
            }
            Ok(Payload::Log(entry.log.iter().cloned().collect()))
        }
        Request::SystemSettings => Ok(Payload::Settings(daemon.settings.read().await.clone())),
        Request::SetSystemSettings { settings } => {
            if uid != ROOT {
                return Err("administrator rights are required".into());
            }
            daemon.save_settings(settings).await.map_err(|e| e.to_string())?;
            Ok(Payload::Empty {})
        }
        Request::Blocked => Ok(Payload::Blocked(daemon.blocked(uid).await)),
        Request::Unblock { id } => {
            daemon.release_kill_switch(uid, id.as_deref()).await;
            Ok(Payload::Empty {})
        }
        Request::Versions => {
            let binaries = tokio::task::spawn_blocking(openvpn::discover).await.unwrap_or_default();
            Ok(Payload::Versions(binaries.into_iter().map(|b| b.version).collect()))
        }
    }
}
