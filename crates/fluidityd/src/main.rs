mod daemon;
mod killswitch;
mod net;
mod netconfig;
mod obfs;
mod openvpn;
mod proxy;
mod server;
mod session;
mod usage;
#[cfg(windows)]
mod windows;

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
    #[cfg(windows)]
    #[arg(long, help = "Run as a Windows service")]
    service: bool,
}

fn main() {
    let args = Args::parse();
    #[cfg(windows)]
    if args.service {
        let log = Paths::system().data_dir.join("fluidityd.log");
        init_tracing(&args.log, Some(&log));
        if let Err(err) = windows::run_service() {
            tracing::error!("service: {err}");
            std::process::exit(1);
        }
        return;
    }
    init_tracing(&args.log, None);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(run(args.socket, interrupted()));
}

fn init_tracing(filter: &str, file: Option<&std::path::Path>) {
    let builder = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_target(false);
    let opened = file.and_then(|path| {
        let _ = std::fs::create_dir_all(path.parent()?);
        std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
    });
    match opened {
        Some(file) => builder.with_ansi(false).with_writer(std::sync::Mutex::new(file)).init(),
        None => builder
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
            .with_writer(std::io::stderr)
            .init(),
    }
}

#[cfg(unix)]
async fn interrupted() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

#[cfg(windows)]
async fn interrupted() {
    let _ = tokio::signal::ctrl_c().await;
}

pub async fn run(socket: Option<PathBuf>, shutdown: impl std::future::Future<Output = ()>) {
    let paths = Paths::system();
    for dir in [&paths.run_dir, &paths.data_dir] {
        if let Err(err) = std::fs::create_dir_all(dir) {
            tracing::error!("{}: {err}", dir.display());
            std::process::exit(1);
        }
    }
    #[cfg(windows)]
    windows::secure_dir(&paths.data_dir);
    let backend = net::backend(&paths.net_state_dir());
    backend.recover();
    let daemon = Daemon::new(paths, backend);

    let socket = socket.unwrap_or_else(fluidity_ipc::socket_path);
    #[cfg(unix)]
    let result = listen(daemon.clone(), &socket, shutdown).await;
    #[cfg(windows)]
    let result = windows::listen(daemon.clone(), &socket, shutdown).await;
    if let Err(err) = result {
        tracing::error!("{}: {err}", socket.display());
        std::process::exit(1);
    }
}

pub(crate) async fn shutdown(daemon: Arc<Daemon>) {
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
async fn listen(
    daemon: Arc<Daemon>,
    socket: &std::path::Path,
    stop: impl std::future::Future<Output = ()>,
) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(socket);
    let listener = tokio::net::UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o666))?;
    tracing::info!("listening on {}", socket.display());

    tokio::pin!(stop);
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
            _ = &mut stop => break,
        }
    }
    tracing::info!("shutting down");
    shutdown(daemon).await;
    let _ = std::fs::remove_file(socket);
    Ok(())
}
