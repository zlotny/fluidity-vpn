use std::time::Duration;

use fluidity_ipc::{Client, ConnectOptions, Event, LogEntry, Payload, Reply, Request, SessionInfo, WireBundle};

use crate::tray::{self, TrayAction, TraySnapshot};

pub enum Command {
    Connect {
        id: String,
        bundle: WireBundle,
        options: ConnectOptions,
    },
    Disconnect(String),
    DisconnectAll,
    Reply {
        id: String,
        reply: Reply,
    },
    Log(String),
    Versions,
    Unblock(Option<String>),
    Tray(TraySnapshot),
}

pub enum Message {
    Daemon(bool),
    Sessions(Vec<SessionInfo>),
    Event(Box<Event>),
    Log(String, Vec<LogEntry>),
    Versions(Vec<String>),
    Blocked(Vec<String>),
    Error(String),
    Tray(TrayAction),
}

pub fn start(ui: async_channel::Sender<Message>) -> async_channel::Sender<Command> {
    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        runtime.block_on(run(ui, rx));
    });
    tx
}

async fn run(ui: async_channel::Sender<Message>, commands: async_channel::Receiver<Command>) {
    let tray = tray::spawn(ui.clone()).await;
    let mut up = None;
    loop {
        let connected = Client::connect(&fluidity_ipc::socket_path()).await;
        let (client, mut events) = match connected {
            Ok(pair) => pair,
            Err(_) => {
                if up != Some(false) {
                    up = Some(false);
                    let _ = ui.send(Message::Daemon(false)).await;
                }
                let wait = tokio::time::sleep(Duration::from_secs(2));
                tokio::pin!(wait);
                loop {
                    tokio::select! {
                        _ = &mut wait => break,
                        command = commands.recv() => match command {
                            Ok(Command::Tray(snapshot)) => update_tray(&tray, snapshot).await,
                            Ok(_) => {
                                let _ = ui.send(Message::Error("The Fluidity service is not running.".into())).await;
                            }
                            Err(_) => return,
                        },
                    }
                }
                continue;
            }
        };
        if client.subscribe().await.is_err() {
            continue;
        }
        up = Some(true);
        let _ = ui.send(Message::Daemon(true)).await;
        if let Ok(sessions) = client.sessions().await {
            let _ = ui.send(Message::Sessions(sessions)).await;
        }
        if let Ok(Payload::Blocked(ids)) = client.request(Request::Blocked).await {
            let _ = ui.send(Message::Blocked(ids)).await;
        }
        loop {
            tokio::select! {
                event = events.recv() => match event {
                    Some(event) => {
                        let _ = ui.send(Message::Event(Box::new(event))).await;
                    }
                    None => break,
                },
                command = commands.recv() => match command {
                    Ok(Command::Tray(snapshot)) => update_tray(&tray, snapshot).await,
                    Ok(command) => {
                        if let Err(error) = execute(&client, &ui, command).await {
                            let _ = ui.send(Message::Error(error)).await;
                        }
                    }
                    Err(_) => return,
                },
            }
        }
    }
}

async fn update_tray(tray: &Option<tray::Handle>, snapshot: TraySnapshot) {
    if let Some(handle) = tray {
        handle.update(snapshot).await;
    }
}

async fn execute(client: &Client, ui: &async_channel::Sender<Message>, command: Command) -> Result<(), String> {
    let request = match command {
        Command::Connect { id, bundle, options } => Request::Connect { id, bundle, options },
        Command::Disconnect(id) => Request::Disconnect { id },
        Command::DisconnectAll => Request::DisconnectAll,
        Command::Unblock(id) => Request::Unblock { id },
        Command::Reply { id, reply } => return client.reply(&id, reply).await.map_err(|e| e.to_string()),
        Command::Log(id) => {
            let entries = client.log(&id).await.map_err(|e| e.to_string())?;
            let _ = ui.send(Message::Log(id, entries)).await;
            return Ok(());
        }
        Command::Versions => {
            if let Payload::Versions(list) = client.request(Request::Versions).await.map_err(|e| e.to_string())? {
                let _ = ui.send(Message::Versions(list)).await;
            }
            return Ok(());
        }
        Command::Tray(_) => return Ok(()),
    };
    client.request(request).await.map(|_| ()).map_err(|e| e.to_string())
}
