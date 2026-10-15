use fluidity_ipc::SessionState;

#[cfg(target_os = "linux")]
mod sni;
#[cfg(target_os = "linux")]
pub use sni::{Handle, spawn};
#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::{Handle, spawn};

#[derive(Debug, Clone)]
pub enum TrayAction {
    Toggle(String),
    ConnectAll,
    DisconnectAll,
    Show,
    Preferences,
    RunItem(String),
    Import,
    Quit,
}

#[derive(Debug, Clone, Default)]
pub struct TrayConnection {
    pub id: String,
    pub name: String,
    pub state: SessionState,
}

#[derive(Debug, Clone)]
pub enum TrayNode {
    Connection(TrayConnection),
    Folder(String, Vec<TrayNode>),
}

#[derive(Debug, Clone, Default)]
pub struct TraySnapshot {
    pub nodes: Vec<TrayNode>,
    pub stats: Vec<String>,
    pub daemon: bool,
    #[cfg_attr(windows, allow(dead_code))]
    pub icon: String,
    pub items: Vec<(String, String)>,
}

impl TraySnapshot {
    pub fn overall(&self) -> SessionState {
        fn walk(nodes: &[TrayNode], out: &mut Vec<SessionState>) {
            for node in nodes {
                match node {
                    TrayNode::Connection(c) => out.push(c.state),
                    TrayNode::Folder(_, children) => walk(children, out),
                }
            }
        }
        let mut states = Vec::new();
        walk(&self.nodes, &mut states);
        if states.contains(&SessionState::Connected) {
            SessionState::Connected
        } else if states.iter().any(|s| s.is_active()) {
            SessionState::Connecting
        } else if states.contains(&SessionState::Error) {
            SessionState::Error
        } else {
            SessionState::Disconnected
        }
    }
}

pub fn state_label(state: SessionState) -> &'static str {
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
