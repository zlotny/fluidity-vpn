use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use adw::prelude::*;
use fluidity_core::{CredentialStore, Credentials, Node, Preferences, Store, wire_bundle};
use fluidity_ipc::{ConnectOptions, Event, LogEntry, Prompt, Reply, SessionInfo, SessionState};
use gtk::{gio, glib};

use crate::backend::{Command, Message};
use crate::state::{self, ConnInfo, TrafficSample};
use crate::tray::{TrayAction, TrayConnection, TrayNode, TraySnapshot};
use crate::window::Window;
use crate::{prompt, scripts};

pub struct App {
    pub gtk: adw::Application,
    pub store: Store,
    pub prefs: RefCell<Preferences>,
    pub tree: RefCell<Vec<Node>>,
    pub connections: RefCell<Vec<ConnInfo>>,
    pub sessions: RefCell<HashMap<String, SessionInfo>>,
    pub logs: RefCell<HashMap<String, Vec<LogEntry>>>,
    pub traffic: RefCell<HashMap<String, Vec<TrafficSample>>>,
    pub daemon: Cell<bool>,
    pub versions: RefCell<Vec<String>>,
    backend: async_channel::Sender<Command>,
    window: RefCell<Option<Rc<Window>>>,
    usernames: RefCell<HashMap<String, String>>,
    pending_challenge: RefCell<HashMap<String, String>>,
    user_disconnect: RefCell<HashSet<String>>,
    previous: RefCell<HashMap<String, SessionState>>,
    wanted_before_sleep: RefCell<HashSet<String>>,
    wanted_before_network: RefCell<HashSet<String>>,
    network_up: Cell<bool>,
    launched: Cell<bool>,
    idle_timer: RefCell<Option<glib::SourceId>>,
    pub prompts: RefCell<HashMap<String, adw::AlertDialog>>,
}

impl App {
    pub fn new(gtk: adw::Application, store: Store) -> Rc<Self> {
        let (ui_tx, ui_rx) = async_channel::unbounded();
        let backend = crate::backend::start(ui_tx);
        let prefs = Preferences::load(store.root());
        let app = Rc::new(Self {
            gtk,
            prefs: RefCell::new(prefs),
            store,
            tree: RefCell::default(),
            connections: RefCell::default(),
            sessions: RefCell::default(),
            logs: RefCell::default(),
            traffic: RefCell::default(),
            daemon: Cell::new(false),
            versions: RefCell::default(),
            backend,
            window: RefCell::default(),
            usernames: RefCell::default(),
            pending_challenge: RefCell::default(),
            user_disconnect: RefCell::default(),
            previous: RefCell::default(),
            wanted_before_sleep: RefCell::default(),
            wanted_before_network: RefCell::default(),
            network_up: Cell::new(true),
            launched: Cell::new(false),
            idle_timer: RefCell::default(),
            prompts: RefCell::default(),
        });
        app.reload();
        let weak = Rc::downgrade(&app);
        glib::spawn_future_local(async move {
            while let Ok(message) = ui_rx.recv().await {
                let Some(app) = weak.upgrade() else { break };
                app.handle(message);
            }
        });
        app.observe_system();
        app.schedule_idle_check();
        app
    }

    pub fn send(&self, command: Command) {
        let _ = self.backend.try_send(command);
    }

    pub fn window(self: &Rc<Self>) -> Rc<Window> {
        if let Some(w) = self.window.borrow().as_ref() {
            return w.clone();
        }
        let w = Window::new(self);
        *self.window.borrow_mut() = Some(w.clone());
        w
    }

    pub fn existing_window(&self) -> Option<Rc<Window>> {
        self.window.borrow().clone()
    }

    pub fn parent(self: &Rc<Self>) -> adw::ApplicationWindow {
        self.window().win.clone()
    }

    pub fn show(self: &Rc<Self>) {
        self.window().win.present();
    }

    pub fn reload(&self) {
        let (tree, list) = state::load(&self.store);
        *self.tree.borrow_mut() = tree;
        *self.connections.borrow_mut() = list;
        self.refresh_all();
    }

    pub fn connection(&self, id: &str) -> Option<ConnInfo> {
        self.connections.borrow().iter().find(|c| c.id == id).cloned()
    }

    pub fn state_of(&self, id: &str) -> SessionState {
        self.sessions.borrow().get(id).map(|s| s.state).unwrap_or_default()
    }

    pub fn is_active(&self, id: &str) -> bool {
        self.state_of(id).is_active()
    }

    pub fn set_tree(&self, tree: Vec<Node>) {
        if let Err(e) = self.store.set_tree(&tree) {
            self.error(&e.to_string());
        }
        self.reload();
    }

    pub fn save_prefs(self: &Rc<Self>) {
        if let Err(e) = self.prefs.borrow().save(self.store.root()) {
            self.error(&e.to_string());
        }
        self.schedule_idle_check();
        self.refresh_all();
    }

    pub fn error(&self, message: &str) {
        if let Some(w) = self.existing_window() {
            w.toast(message);
        } else {
            self.notify("Fluidity", message);
        }
    }

    pub fn notify(&self, title: &str, body: &str) {
        if !self.prefs.borrow().notifications {
            return;
        }
        let n = gio::Notification::new(title);
        n.set_body(Some(body));
        n.set_default_action("app.show");
        self.gtk.send_notification(None, &n);
    }

    fn refresh_all(&self) {
        if let Some(w) = self.existing_window() {
            w.rebuild();
        }
        self.update_tray();
    }

    fn refresh_sessions(&self) {
        if let Some(w) = self.existing_window() {
            w.refresh();
        }
        self.update_tray();
    }

    fn update_tray(&self) {
        fn nodes(app: &App, list: &[Node]) -> Vec<TrayNode> {
            list.iter()
                .filter_map(|n| match n {
                    Node::Connection(id) => app.connection(id).map(|c| {
                        TrayNode::Connection(TrayConnection {
                            state: app.state_of(id),
                            id: c.id,
                            name: c.name,
                        })
                    }),
                    Node::Folder(f) => Some(TrayNode::Folder(f.name.clone(), nodes(app, &f.children))),
                })
                .collect()
        }
        let prefs = self.prefs.borrow();
        let mut stats = Vec::new();
        for c in self.connections.borrow().iter() {
            let sessions = self.sessions.borrow();
            let Some(s) = sessions.get(&c.id).filter(|s| s.state == SessionState::Connected) else {
                continue;
            };
            let address = match prefs.address_display {
                fluidity_core::AddressDisplay::Ipv6 if !s.local_ipv6.is_empty() => &s.local_ipv6,
                _ => &s.local_ipv4,
            };
            stats.push(format!("{}: {address}", c.name));
            let line = match prefs.traffic_display {
                fluidity_core::TrafficDisplay::Totals => format!(
                    "In {}  Out {}",
                    state::bytes(s.bytes_in as f64),
                    state::bytes(s.bytes_out as f64)
                ),
                fluidity_core::TrafficDisplay::Rates => {
                    let traffic = self.traffic.borrow();
                    let last = traffic.get(&c.id).and_then(|t| t.last().copied());
                    let (i, o) = last.map(|t| (t.in_rate, t.out_rate)).unwrap_or_default();
                    format!("In {}/s  Out {}/s", state::bytes(i), state::bytes(o))
                }
            };
            stats.push(line);
        }
        let snapshot = TraySnapshot {
            nodes: nodes(self, &self.tree.borrow()),
            stats,
            daemon: self.daemon.get(),
            icon: prefs.icon_set.clone(),
            items: crate::scripts::menu_items(self.store.root()),
        };
        self.send(Command::Tray(snapshot));
    }

    fn handle(self: &Rc<Self>, message: Message) {
        match message {
            Message::Daemon(up) => {
                self.daemon.set(up);
                if up {
                    self.send(Command::Versions);
                }
                self.refresh_sessions();
            }
            Message::Sessions(list) => {
                {
                    let mut previous = self.previous.borrow_mut();
                    for s in &list {
                        previous.insert(s.id.clone(), s.state);
                    }
                }
                *self.sessions.borrow_mut() = list.into_iter().map(|s| (s.id.clone(), s)).collect();
                if !self.launched.replace(true) {
                    let wanted: Vec<String> = self
                        .connections
                        .borrow()
                        .iter()
                        .filter(|c| c.connect_on_launch)
                        .map(|c| c.id.clone())
                        .collect();
                    for id in wanted {
                        if !self.sessions.borrow().contains_key(&id) {
                            self.connect(&id);
                        }
                    }
                }
                self.refresh_sessions();
            }
            Message::Event(event) => self.event(*event),
            Message::Log(id, entries) => {
                self.logs.borrow_mut().insert(id.clone(), entries);
                if let Some(w) = self.existing_window() {
                    w.log_changed(&id);
                }
            }
            Message::Versions(list) => *self.versions.borrow_mut() = list,
            Message::Error(e) => self.error(&e),
            Message::Tray(action) => self.tray_action(action),
        }
    }

    fn tray_action(self: &Rc<Self>, action: TrayAction) {
        match action {
            TrayAction::Toggle(id) => self.toggle(&id),
            TrayAction::ConnectAll => self.connect_all(),
            TrayAction::DisconnectAll => self.disconnect_all(),
            TrayAction::Show => self.show(),
            TrayAction::Preferences => crate::prefs::open(self),
            TrayAction::Import => self.window().import(),
            TrayAction::Quit => self.gtk.quit(),
            TrayAction::RunItem(path) => self.run_menu_item(path),
        }
    }

    fn event(self: &Rc<Self>, event: Event) {
        match event {
            Event::Session { session } => self.update(session),
            Event::Removed { id } => {
                let old = self.sessions.borrow_mut().remove(&id);
                if let Some(old) = old {
                    self.transition(&old, SessionState::Disconnected, None);
                }
                self.previous.borrow_mut().remove(&id);
                self.traffic.borrow_mut().remove(&id);
                prompt::dismiss(self, &id);
                self.refresh_sessions();
            }
            Event::Prompt { id, prompt } => self.answer(&id, prompt),
            Event::PromptDone { id } => prompt::dismiss(self, &id),
            Event::Log { id, entry } => {
                {
                    let mut logs = self.logs.borrow_mut();
                    let list = logs.entry(id.clone()).or_default();
                    list.push(entry.clone());
                    if list.len() > 5000 {
                        list.drain(..list.len() - 5000);
                    }
                }
                if let Some(w) = self.existing_window() {
                    w.log_appended(&id, &entry);
                }
            }
        }
    }

    fn update(self: &Rc<Self>, info: SessionInfo) {
        let old = self.sessions.borrow_mut().insert(info.id.clone(), info.clone());
        if let Some(old) = &old
            && old.state == SessionState::Connected
            && info.state == SessionState::Connected
        {
            let now = state::now();
            let mut traffic = self.traffic.borrow_mut();
            let history = traffic.entry(info.id.clone()).or_default();
            let dt = history.last().map(|t| now - t.time).unwrap_or(1.0).max(0.2);
            history.push(TrafficSample {
                time: now,
                in_rate: info.bytes_in.saturating_sub(old.bytes_in) as f64 / dt,
                out_rate: info.bytes_out.saturating_sub(old.bytes_out) as f64 / dt,
            });
            if history.len() > state::HISTORY {
                history.drain(..history.len() - state::HISTORY);
            }
        }
        let previous = self
            .previous
            .borrow_mut()
            .insert(info.id.clone(), info.state)
            .unwrap_or_default();
        if previous != info.state {
            let mut before = old.unwrap_or_else(|| info.clone());
            before.state = previous;
            self.transition(&before, info.state, Some(&info));
        }
        self.refresh_sessions();
    }

    fn transition(self: &Rc<Self>, old: &SessionInfo, state: SessionState, current: Option<&SessionInfo>) {
        let Some(connection) = self.connection(&old.id) else {
            return;
        };
        let info = current.unwrap_or(old);
        match state {
            SessionState::Connected if old.state != SessionState::Connected => {
                let suffix = if info.local_ipv4.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", info.local_ipv4)
                };
                self.notify(&connection.name, &format!("Connected{suffix}"));
                self.run_script(connection.connected_script.clone(), &connection, info);
            }
            SessionState::Disconnected | SessionState::Error => {
                if old.state == SessionState::Connected {
                    if self.user_disconnect.borrow().contains(&old.id) || state == SessionState::Error {
                        self.notify(&connection.name, info.last_error.as_deref().unwrap_or("Disconnected"));
                    }
                    self.run_script(connection.disconnected_script.clone(), &connection, old);
                } else if state == SessionState::Error
                    && let Some(e) = &info.last_error
                {
                    self.notify(&connection.name, e);
                }
                self.user_disconnect.borrow_mut().remove(&old.id);
            }
            SessionState::Reconnecting if old.state == SessionState::Connected => {
                self.notify(&connection.name, "Connection lost, reconnecting");
            }
            _ => {}
        }
    }

    fn run_script(&self, path: Option<String>, connection: &ConnInfo, session: &SessionInfo) {
        let Some(path) = path else { return };
        let env = scripts::environment(
            connection,
            Some(session),
            self.usernames.borrow().get(&connection.id).map(String::as_str),
        );
        std::thread::spawn(move || scripts::run(&path, &env, std::time::Duration::from_secs(120)));
    }

    fn run_menu_item(&self, path: String) {
        let sessions = self.sessions.borrow();
        let session = sessions.values().find(|s| s.state == SessionState::Connected);
        let env = match session.and_then(|s| self.connection(&s.id)) {
            Some(c) => scripts::environment(&c, session, self.usernames.borrow().get(&c.id).map(String::as_str)),
            None => std::env::vars().collect(),
        };
        std::thread::spawn(move || scripts::run(&path, &env, std::time::Duration::from_secs(120)));
    }

    pub fn append_local_log(&self, id: &str, level: &str, message: &str) {
        let entry = LogEntry {
            time: state::now() as u64,
            level: level.into(),
            message: message.into(),
        };
        self.logs
            .borrow_mut()
            .entry(id.to_string())
            .or_default()
            .push(entry.clone());
        if let Some(w) = self.existing_window() {
            w.log_appended(id, &entry);
        }
    }

    pub fn toggle(self: &Rc<Self>, id: &str) {
        if self.is_active(id) {
            self.disconnect(id);
        } else {
            self.connect(id);
        }
    }

    pub fn connect(self: &Rc<Self>, id: &str) {
        let Some(connection) = self.connection(id) else { return };
        if !self.daemon.get() {
            self.error("The Fluidity service is not running. Start it with: sudo systemctl enable --now fluidityd");
            return;
        }
        self.user_disconnect.borrow_mut().remove(id);
        let app = self.clone();
        glib::spawn_future_local(async move {
            if let Some(script) = connection.before_connect_script.clone() {
                let c = connection.clone();
                let result = gio::spawn_blocking(move || scripts::before_connect(&script, &c))
                    .await
                    .unwrap_or_default();
                if !result.allow {
                    app.append_local_log(
                        &connection.id,
                        "warning",
                        "Before-connect script cancelled the connection",
                    );
                    return;
                }
                if let Some(u) = &result.username {
                    app.usernames.borrow_mut().insert(connection.id.clone(), u.clone());
                }
                if result.username.is_some() || result.password.is_some() {
                    let _ = CredentialStore::save(
                        &format!("script:{}", connection.id),
                        &Credentials {
                            username: result.username.clone(),
                            password: result.password.clone(),
                        },
                    );
                }
                if let Some(r) = result.challenge {
                    app.pending_challenge.borrow_mut().insert(connection.id.clone(), r);
                }
            }
            let bundle = match app.store.load(&connection.id) {
                Ok(b) => b,
                Err(e) => {
                    app.report(&connection, &e.to_string());
                    return;
                }
            };
            let prefs = app.prefs.borrow().clone();
            app.send(Command::Connect {
                id: connection.id.clone(),
                bundle: wire_bundle(&bundle),
                options: ConnectOptions {
                    openvpn_version: connection.openvpn_version.clone().or(prefs.openvpn_version),
                    auto_reconnect: connection.auto_reconnect,
                    block_ipv6: prefs.block_ipv6,
                    disable_time_machine: false,
                },
            });
        });
    }

    fn report(&self, connection: &ConnInfo, message: &str) {
        self.append_local_log(&connection.id, "error", message);
        self.error(message);
    }

    pub fn disconnect(self: &Rc<Self>, id: &str) {
        self.user_disconnect.borrow_mut().insert(id.to_string());
        self.wanted_before_sleep.borrow_mut().remove(id);
        self.wanted_before_network.borrow_mut().remove(id);
        prompt::dismiss(self, id);
        self.send(Command::Disconnect(id.to_string()));
    }

    pub fn connect_all(self: &Rc<Self>) {
        let ids: Vec<String> = self.connections.borrow().iter().map(|c| c.id.clone()).collect();
        for id in ids {
            if !self.is_active(&id) {
                self.connect(&id);
            }
        }
    }

    pub fn disconnect_all(self: &Rc<Self>) {
        for (id, s) in self.sessions.borrow().iter() {
            if s.state.is_active() {
                self.user_disconnect.borrow_mut().insert(id.clone());
            }
        }
        prompt::dismiss_all(self);
        self.send(Command::DisconnectAll);
    }

    pub fn active_ids(&self) -> Vec<String> {
        self.sessions
            .borrow()
            .values()
            .filter(|s| s.state.is_active())
            .map(|s| s.id.clone())
            .collect()
    }

    fn scope(connection: &ConnInfo, realm: &str) -> String {
        if realm == "Auth" {
            connection.credential_scope.clone()
        } else {
            format!("{}:{}", connection.credential_scope, state::secret_field(realm))
        }
    }

    fn saved(connection: &ConnInfo, realm: &str) -> Credentials {
        if realm == "Auth" {
            let script_scope = format!("script:{}", connection.id);
            let scripted = CredentialStore::load(&script_scope);
            if scripted.username.is_some() || scripted.password.is_some() {
                CredentialStore::clear(&script_scope);
                return scripted;
            }
        }
        CredentialStore::load(&Self::scope(connection, realm))
    }

    pub fn reply(&self, id: &str, reply: Reply) {
        if reply == Reply::Cancel {
            self.user_disconnect.borrow_mut().insert(id.to_string());
        }
        self.send(Command::Reply {
            id: id.to_string(),
            reply,
        });
    }

    fn answer(self: &Rc<Self>, id: &str, prompt: Prompt) {
        if matches!(prompt, Prompt::Certificate { .. } | Prompt::Sign { .. }) {
            if let Some(c) = self.connection(id) {
                self.report(&c, "System certificate identities are not supported on Linux.");
            }
            self.reply(id, Reply::Cancel);
            return;
        }
        let Some(connection) = self.connection(id) else {
            let title = self
                .sessions
                .borrow()
                .get(id)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            let app = self.clone();
            let id_owned = id.to_string();
            prompt::present(
                self,
                id,
                &title,
                prompt,
                String::new(),
                None,
                Box::new(move |reply, _| app.reply(&id_owned, reply)),
            );
            return;
        };
        if let Prompt::Credentials {
            realm,
            static_challenge,
            retry: false,
        } = &prompt
        {
            let saved = Self::saved(&connection, realm);
            if let (Some(username), Some(password)) = (saved.username.clone(), saved.password.clone())
                && !password.is_empty()
            {
                let response = match static_challenge {
                    Some(challenge) => {
                        let scripted = self.pending_challenge.borrow_mut().remove(id);
                        match scripted.or_else(|| connection.static_challenge_acknowledge.clone()) {
                            Some(r) => Some(r),
                            None => {
                                self.challenge_only(id, &connection, realm, username, password, challenge);
                                return;
                            }
                        }
                    }
                    None => None,
                };
                self.usernames.borrow_mut().insert(id.to_string(), username.clone());
                self.reply(
                    id,
                    Reply::Credentials {
                        realm: realm.clone(),
                        username,
                        password,
                        challenge_response: response,
                    },
                );
                return;
            }
        }
        if let Prompt::Password { realm } = &prompt
            && let Some(password) = CredentialStore::secret(&connection.credential_scope, &state::secret_field(realm))
            && !password.is_empty()
        {
            self.reply(
                id,
                Reply::Password {
                    realm: realm.clone(),
                    password,
                },
            );
            return;
        }
        let mut username = String::new();
        let mut splitter = None;
        if let Prompt::Credentials { realm, .. } = &prompt {
            username = Self::saved(&connection, realm).username.unwrap_or_default();
            if realm == "Auth" {
                splitter = connection.static_challenge_password.clone();
            }
        }
        let app = self.clone();
        let id_owned = id.to_string();
        let is_password = matches!(prompt, Prompt::Password { .. });
        let title = connection.name.clone();
        prompt::present(
            self,
            id,
            &title,
            prompt,
            username,
            splitter,
            Box::new(move |reply, remember| {
                match &reply {
                    Reply::Credentials {
                        realm,
                        username,
                        password,
                        ..
                    } => {
                        if realm == "Auth" {
                            app.usernames.borrow_mut().insert(id_owned.clone(), username.clone());
                        }
                        if remember {
                            let saved = Credentials {
                                username: Some(username.clone()),
                                password: Some(password.clone()),
                            };
                            if let Err(e) = CredentialStore::save(&Self::scope(&connection, realm), &saved) {
                                app.error(&format!("Cannot save credentials: {e}"));
                            }
                        }
                    }
                    Reply::Password { realm, password } if remember && is_password => {
                        if let Err(e) = CredentialStore::save_secret(
                            &connection.credential_scope,
                            &state::secret_field(realm),
                            password,
                        ) {
                            app.error(&format!("Cannot save password: {e}"));
                        }
                    }
                    _ => {}
                }
                app.reply(&id_owned, reply);
            }),
        );
    }

    fn challenge_only(
        self: &Rc<Self>,
        id: &str,
        connection: &ConnInfo,
        realm: &str,
        username: String,
        password: String,
        challenge: &fluidity_ipc::StaticChallengePrompt,
    ) {
        let app = self.clone();
        let id_owned = id.to_string();
        let realm_owned = realm.to_string();
        prompt::present(
            self,
            id,
            &connection.name,
            Prompt::Challenge {
                realm: realm.into(),
                text: challenge.text.clone(),
                echo: challenge.echo,
            },
            String::new(),
            None,
            Box::new(move |reply, _| match reply {
                Reply::Password { password: response, .. } => {
                    app.usernames.borrow_mut().insert(id_owned.clone(), username.clone());
                    app.reply(
                        &id_owned,
                        Reply::Credentials {
                            realm: realm_owned.clone(),
                            username: username.clone(),
                            password: password.clone(),
                            challenge_response: Some(response),
                        },
                    );
                }
                other => app.reply(&id_owned, other),
            }),
        );
    }

    pub fn forget(&self, id: &str) {
        if let Some(c) = self.connection(id) {
            CredentialStore::clear(&c.credential_scope);
            CredentialStore::clear(&c.id);
        }
    }

    pub fn remove(self: &Rc<Self>, id: &str) {
        if self.is_active(id) {
            self.disconnect(id);
        }
        self.forget(id);
        if let Err(e) = self.store.remove(id) {
            self.error(&e.to_string());
        }
        self.reload();
    }

    pub fn duplicate(&self, id: &str) -> Option<String> {
        let result = self.store.duplicate(id);
        self.reload();
        match result {
            Ok(c) => Some(c.id),
            Err(e) => {
                self.error(&e.to_string());
                None
            }
        }
    }

    pub fn import_paths(self: &Rc<Self>, paths: &[std::path::PathBuf]) -> Option<String> {
        let mut names = Vec::new();
        let mut last = None;
        for path in paths {
            match self.store.import(path) {
                Ok(c) => {
                    names.push(c.name);
                    last = Some(c.id);
                }
                Err(e) => self.error(&format!(
                    "Cannot import {}: {e}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )),
            }
        }
        self.reload();
        if !names.is_empty() {
            self.notify("Imported", &names.join(", "));
        }
        last
    }

    fn observe_system(self: &Rc<Self>) {
        let monitor = gio::NetworkMonitor::default();
        self.network_up.set(monitor.is_network_available());
        let weak = Rc::downgrade(self);
        monitor.connect_network_changed(move |_, available| {
            if let Some(app) = weak.upgrade() {
                app.network_changed(available);
            }
        });
        let weak = Rc::downgrade(self);
        gio::bus_get(gio::BusType::System, gio::Cancellable::NONE, move |bus| {
            let Ok(bus) = bus else { return };
            let weak = weak.clone();
            let subscription = bus.subscribe_to_signal(
                Some("org.freedesktop.login1"),
                Some("org.freedesktop.login1.Manager"),
                Some("PrepareForSleep"),
                Some("/org/freedesktop/login1"),
                None,
                gio::DBusSignalFlags::NONE,
                move |signal| {
                    let Some((sleeping,)) = signal.parameters.get::<(bool,)>() else {
                        return;
                    };
                    if let Some(app) = weak.upgrade() {
                        app.sleep_changed(sleeping);
                    }
                },
            );
            std::mem::forget(subscription);
        });
    }

    fn sleep_changed(self: &Rc<Self>, sleeping: bool) {
        if !self.prefs.borrow().reconnect_on_wake {
            return;
        }
        if sleeping {
            *self.wanted_before_sleep.borrow_mut() = self.active_ids().into_iter().collect();
            return;
        }
        let wanted = std::mem::take(&mut *self.wanted_before_sleep.borrow_mut());
        let app = self.clone();
        glib::timeout_add_seconds_local_once(3, move || {
            for id in wanted {
                if !app.is_active(&id) {
                    app.connect(&id);
                }
            }
        });
    }

    fn network_changed(self: &Rc<Self>, available: bool) {
        let was = self.network_up.replace(available);
        if !self.prefs.borrow().reconnect_on_network || was == available {
            return;
        }
        if !available {
            *self.wanted_before_network.borrow_mut() = self.active_ids().into_iter().collect();
        } else {
            let wanted = std::mem::take(&mut *self.wanted_before_network.borrow_mut());
            for id in wanted {
                if !self.is_active(&id) {
                    self.connect(&id);
                }
            }
        }
    }

    fn schedule_idle_check(self: &Rc<Self>) {
        if let Some(source) = self.idle_timer.borrow_mut().take() {
            source.remove();
        }
        let Some(minutes) = self.prefs.borrow().idle_disconnect_minutes.filter(|m| *m > 0) else {
            return;
        };
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_seconds_local(30, move || {
            let Some(app) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            app.check_idle(minutes);
            glib::ControlFlow::Continue
        });
        *self.idle_timer.borrow_mut() = Some(source);
    }

    fn check_idle(self: &Rc<Self>, minutes: u32) {
        let limit = minutes as f64 * 60.0;
        let now = state::now();
        let idle: Vec<String> = self
            .sessions
            .borrow()
            .values()
            .filter(|s| s.state == SessionState::Connected)
            .filter(|s| s.connected_since.is_some_and(|since| now - since as f64 >= limit))
            .filter(|s| {
                let traffic = self.traffic.borrow();
                let Some(samples) = traffic.get(&s.id) else {
                    return false;
                };
                !samples
                    .iter()
                    .filter(|t| now - t.time < limit)
                    .any(|t| t.in_rate > 64.0 || t.out_rate > 64.0)
            })
            .map(|s| s.id.clone())
            .collect();
        for id in idle {
            self.append_local_log(&id, "info", &format!("Disconnecting after {minutes} idle minutes"));
            self.disconnect(&id);
        }
    }
}
