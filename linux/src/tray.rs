use fluidity_ipc::SessionState;
use ksni::TrayMethods;
use ksni::menu::{MenuItem, StandardItem, SubMenu};

use crate::backend::Message;

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
    pub icon: String,
    pub items: Vec<(String, String)>,
}

pub struct Tray {
    pub snapshot: TraySnapshot,
    ui: async_channel::Sender<Message>,
}

impl Tray {
    fn send(&self, action: TrayAction) {
        let _ = self.ui.try_send(Message::Tray(action));
    }

    fn overall(&self) -> SessionState {
        fn walk(nodes: &[TrayNode], out: &mut Vec<SessionState>) {
            for node in nodes {
                match node {
                    TrayNode::Connection(c) => out.push(c.state),
                    TrayNode::Folder(_, children) => walk(children, out),
                }
            }
        }
        let mut states = Vec::new();
        walk(&self.snapshot.nodes, &mut states);
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

pub fn icon_name(set: &str, state: SessionState) -> String {
    let base = match state {
        SessionState::Connected => "network-vpn-symbolic",
        SessionState::Disconnected | SessionState::Error => "network-vpn-disconnected-symbolic",
        _ => "network-vpn-acquiring-symbolic",
    };
    match set {
        "lock" => match state {
            SessionState::Connected => "changes-prevent-symbolic",
            _ => "changes-allow-symbolic",
        }
        .into(),
        _ => base.into(),
    }
}

fn items(nodes: &[TrayNode]) -> Vec<MenuItem<Tray>> {
    nodes
        .iter()
        .map(|node| match node {
            TrayNode::Connection(c) => {
                let id = c.id.clone();
                let label = if c.state == SessionState::Disconnected {
                    c.name.clone()
                } else {
                    format!("{} ({})", c.name, state_label(c.state))
                };
                StandardItem {
                    label: label.replace('_', "__"),
                    icon_name: icon_name("default", c.state),
                    activate: Box::new(move |t: &mut Tray| t.send(TrayAction::Toggle(id.clone()))),
                    ..Default::default()
                }
                .into()
            }
            TrayNode::Folder(name, children) => SubMenu {
                label: name.replace('_', "__"),
                icon_name: "folder-symbolic".into(),
                submenu: items(children),
                ..Default::default()
            }
            .into(),
        })
        .collect()
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "org.fluidityvpn.Fluidity".into()
    }

    fn title(&self) -> String {
        "Fluidity".into()
    }

    fn icon_name(&self) -> String {
        icon_name(&self.snapshot.icon, self.overall())
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Fluidity".into(),
            description: if self.snapshot.stats.is_empty() {
                "Not connected".into()
            } else {
                self.snapshot.stats.join("\n")
            },
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayAction::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut menu = Vec::new();
        if !self.snapshot.daemon {
            menu.push(
                StandardItem {
                    label: "Fluidity service is not running".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
            menu.push(MenuItem::Separator);
        }
        if self.snapshot.nodes.is_empty() {
            menu.push(
                StandardItem {
                    label: "No Connections".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        } else {
            menu.extend(items(&self.snapshot.nodes));
        }
        if !self.snapshot.stats.is_empty() {
            menu.push(MenuItem::Separator);
            for line in &self.snapshot.stats {
                menu.push(
                    StandardItem {
                        label: line.replace('_', "__"),
                        enabled: false,
                        ..Default::default()
                    }
                    .into(),
                );
            }
        }
        menu.push(MenuItem::Separator);
        let action = |label: &str, action: TrayAction| -> MenuItem<Tray> {
            StandardItem {
                label: label.into(),
                activate: Box::new(move |t: &mut Tray| t.send(action.clone())),
                ..Default::default()
            }
            .into()
        };
        if !self.snapshot.stats.is_empty() {
            menu.push(action("Disconnect All", TrayAction::DisconnectAll));
        }
        menu.push(action("Connect All", TrayAction::ConnectAll));
        if !self.snapshot.items.is_empty() {
            menu.push(MenuItem::Separator);
            for (title, path) in &self.snapshot.items {
                menu.push(action(&title.replace('_', "__"), TrayAction::RunItem(path.clone())));
            }
            menu.push(MenuItem::Separator);
        }
        menu.push(action("Open Fluidity", TrayAction::Show));
        menu.push(action("Preferences", TrayAction::Preferences));
        menu.push(action("Import Connection…", TrayAction::Import));
        menu.push(MenuItem::Separator);
        menu.push(action("Quit Fluidity", TrayAction::Quit));
        menu
    }
}

pub async fn spawn(ui: async_channel::Sender<Message>) -> Option<ksni::Handle<Tray>> {
    let tray = Tray {
        snapshot: TraySnapshot::default(),
        ui,
    };
    tray.spawn().await.ok()
}
