use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Duration;

use fluidity_ipc::SessionState;
use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT, TranslateMessage,
};

use super::{TrayAction, TrayNode, TraySnapshot, state_label};
use crate::backend::Message;

const ICON_PNG: &[u8] = include_bytes!("../../data/icons/fluidity-tray.png");

pub struct Handle(mpsc::Sender<TraySnapshot>);

impl Handle {
    pub async fn update(&self, snapshot: TraySnapshot) {
        let _ = self.0.send(snapshot);
    }
}

pub async fn spawn(ui: async_channel::Sender<Message>) -> Option<Handle> {
    let (tx, rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::spawn(move || run(ui, rx, ready_tx));
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(true) => Some(Handle(tx)),
        _ => None,
    }
}

struct Icons {
    connected: Icon,
    connecting: Icon,
    disconnected: Icon,
}

fn decode() -> Option<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(ICON_PNG));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    Some((buf, info.width, info.height))
}

fn load_icons() -> Option<Icons> {
    let (rgba, w, h) = decode()?;
    let dimmed: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2], (p[3] as u16 * 3 / 5) as u8])
        .collect();
    let gray: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            let y = ((p[0] as u32 * 30 + p[1] as u32 * 59 + p[2] as u32 * 11) / 100) as u8;
            [y, y, y, p[3]]
        })
        .collect();
    Some(Icons {
        connected: Icon::from_rgba(rgba, w, h).ok()?,
        connecting: Icon::from_rgba(dimmed, w, h).ok()?,
        disconnected: Icon::from_rgba(gray, w, h).ok()?,
    })
}

fn escape(text: &str) -> String {
    text.replace('&', "&&")
}

struct Builder {
    actions: HashMap<MenuId, TrayAction>,
}

impl Builder {
    fn item(&mut self, label: &str, action: TrayAction) -> MenuItem {
        let item = MenuItem::new(escape(label), true, None);
        self.actions.insert(item.id().clone(), action);
        item
    }

    fn nodes(&mut self, nodes: &[TrayNode]) -> Vec<Box<dyn IsMenuItem>> {
        nodes
            .iter()
            .map(|node| -> Box<dyn IsMenuItem> {
                match node {
                    TrayNode::Connection(c) => {
                        let label = if c.state == SessionState::Disconnected {
                            c.name.clone()
                        } else {
                            format!("{} ({})", c.name, state_label(c.state))
                        };
                        Box::new(self.item(&label, TrayAction::Toggle(c.id.clone())))
                    }
                    TrayNode::Folder(name, children) => {
                        let sub = Submenu::new(escape(name), true);
                        for child in self.nodes(children) {
                            let _ = sub.append(child.as_ref());
                        }
                        Box::new(sub)
                    }
                }
            })
            .collect()
    }

    fn menu(&mut self, snapshot: &TraySnapshot) -> Menu {
        let menu = Menu::new();
        let separator = || PredefinedMenuItem::separator();
        let disabled = |label: &str| MenuItem::new(escape(label), false, None);
        if !snapshot.daemon {
            let _ = menu.append(&disabled("Fluidity service is not running"));
            let _ = menu.append(&separator());
        }
        if snapshot.nodes.is_empty() {
            let _ = menu.append(&disabled("No Connections"));
        } else {
            for item in self.nodes(&snapshot.nodes) {
                let _ = menu.append(item.as_ref());
            }
        }
        if !snapshot.stats.is_empty() {
            let _ = menu.append(&separator());
            for line in &snapshot.stats {
                let _ = menu.append(&disabled(line));
            }
        }
        let _ = menu.append(&separator());
        if !snapshot.stats.is_empty() {
            let _ = menu.append(&self.item("Disconnect All", TrayAction::DisconnectAll));
        }
        let _ = menu.append(&self.item("Connect All", TrayAction::ConnectAll));
        if !snapshot.items.is_empty() {
            let _ = menu.append(&separator());
            for (title, path) in &snapshot.items {
                let _ = menu.append(&self.item(title, TrayAction::RunItem(path.clone())));
            }
        }
        let _ = menu.append(&separator());
        let _ = menu.append(&self.item("Open Fluidity", TrayAction::Show));
        let _ = menu.append(&self.item("Preferences", TrayAction::Preferences));
        let _ = menu.append(&self.item("Import Connection…", TrayAction::Import));
        let _ = menu.append(&separator());
        let _ = menu.append(&self.item("Quit Fluidity", TrayAction::Quit));
        menu
    }
}

fn tooltip(snapshot: &TraySnapshot) -> String {
    let text = if snapshot.stats.is_empty() {
        "Fluidity\nNot connected".to_string()
    } else {
        format!("Fluidity\n{}", snapshot.stats.join("\n"))
    };
    text.chars().take(127).collect()
}

fn apply(tray: &TrayIcon, icons: &Icons, snapshot: &TraySnapshot) -> HashMap<MenuId, TrayAction> {
    let mut builder = Builder {
        actions: HashMap::new(),
    };
    tray.set_menu(Some(Box::new(builder.menu(snapshot))));
    let icon = match snapshot.overall() {
        SessionState::Connected => &icons.connected,
        SessionState::Disconnected | SessionState::Error => &icons.disconnected,
        _ => &icons.connecting,
    };
    let _ = tray.set_icon(Some(icon.clone()));
    let _ = tray.set_tooltip(Some(tooltip(snapshot)));
    builder.actions
}

fn pump() {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        MsgWaitForMultipleObjects(0, std::ptr::null(), 0, 100, QS_ALLINPUT);
    }
}

fn run(ui: async_channel::Sender<Message>, updates: mpsc::Receiver<TraySnapshot>, ready: mpsc::Sender<bool>) {
    let Some(icons) = load_icons() else {
        let _ = ready.send(false);
        return;
    };
    let tray = TrayIconBuilder::new()
        .with_icon(icons.disconnected.clone())
        .with_tooltip("Fluidity")
        .with_menu_on_left_click(false)
        .build();
    let Ok(tray) = tray else {
        let _ = ready.send(false);
        return;
    };
    let mut actions = apply(&tray, &icons, &TraySnapshot::default());
    let _ = ready.send(true);
    let send = |action: TrayAction| {
        let _ = ui.try_send(Message::Tray(action));
    };
    loop {
        pump();
        loop {
            match updates.try_recv() {
                Ok(snapshot) => actions = apply(&tray, &icons, &snapshot),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some(action) = actions.get(&event.id) {
                send(action.clone());
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                send(TrayAction::Show);
            }
        }
    }
}
