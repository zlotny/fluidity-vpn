mod daemon;
mod net;
mod netconfig;
mod obfs;
mod openvpn;
mod proxy;
mod server;
mod session;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;

use crate::daemon::{Command, Daemon, Paths};

#[derive(Parser)]
#[command(name = "fluidityd", version, about = "Fluidity VPN service")]
struct Args {
    #[arg(long, env = "FLUIDITY_SOCKET")]
    socket: Option<PathBuf>,
    #[arg(long, default_value = "info", env = "FLUIDITY_LOG")]
    log: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(&args.log))
        .with_target(false)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_writer(std::io::stderr)
        .init();

    let paths = Paths::system();
    for dir in [&paths.run_dir, &paths.data_dir] {
        if let Err(err) = std::fs::create_dir_all(dir) {
            tracing::error!("{}: {err}", dir.display());
            std::process::exit(1);
        }
    }
    let backend = net::backend(&paths.net_state_dir());
    backend.recover();
    let daemon = Daemon::new(paths, backend);

    let socket = args.socket.unwrap_or_else(fluidity_ipc::socket_path);
    if let Err(err) = listen(daemon.clone(), &socket).await {
        tracing::error!("{}: {err}", socket.display());
        std::process::exit(1);
    }
}

async fn shutdown(daemon: Arc<Daemon>) {
    {
        let sessions = daemon.sessions.lock().await;
        for entry in sessions.values() {
            let _ = entry.commands.send(Command::Disconnect);
        }
    }
    for _ in 0..120 {
        if daemon.sessions.lock().await.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    daemon.backend.recover();
}

#[cfg(unix)]
async fn listen(daemon: Arc<Daemon>, socket: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::signal::unix::{SignalKind, signal};

    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(socket);
    let listener = tokio::net::UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o666))?;
    tracing::info!("listening on {}", socket.display());

    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(pair) => pair,
                    Err(err) => {
                        tracing::warn!("accept: {err}");
                        continue;
                    }
                };
                let uid = match stream.peer_cred() {
                    Ok(cred) => cred.uid(),
                    Err(err) => {
                        tracing::warn!("peer credentials: {err}");
                        continue;
                    }
                };
                tokio::spawn(server::serve(daemon.clone(), stream, uid));
            }
            _ = term.recv() => break,
            _ = int.recv() => break,
        }
    }
    tracing::info!("shutting down");
    shutdown(daemon).await;
    let _ = std::fs::remove_file(socket);
    Ok(())
}

#[cfg(windows)]
async fn listen(daemon: Arc<Daemon>, pipe: &std::path::Path) -> std::io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let name = pipe.as_os_str();
    let mut server = ServerOptions::new().first_pipe_instance(true).create(name)?;
    tracing::info!("listening on {}", pipe.display());
    loop {
        tokio::select! {
            connected = server.connect() => {
                connected?;
                let client = std::mem::replace(&mut server, ServerOptions::new().create(name)?);
                tokio::spawn(server::serve(daemon.clone(), client, 1000));
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    shutdown(daemon).await;
    Ok(())
}
