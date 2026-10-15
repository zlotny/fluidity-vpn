use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fluidity_config::editor::{EditorModel, FileField, Remote, Route};
use gtk::glib;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::app::App;

type Model = Rc<RefCell<EditorModel>>;
type RouteField = fn(&mut Route) -> &mut String;

struct Editor {
    app: Rc<App>,
    id: Option<String>,
    model: Model,
    dialog: adw::Dialog,
    banner: adw::Banner,
}

pub fn open(app: &Rc<App>, id: Option<String>) {
    let model = match &id {
        Some(id) => match app.store.load(id) {
            Ok(b) => EditorModel::from_document(&b.document, &b.files),
            Err(e) => {
                app.error(&e.to_string());
                return;
            }
        },
        None => EditorModel::new_connection(),
    };
    let dialog = adw::Dialog::builder()
        .title(if id.is_some() {
            "Edit Connection"
        } else {
            "New Connection"
        })
        .content_width(800)
        .content_height(680)
        .width_request(360)
        .height_request(400)
        .build();
    let editor = Rc::new(Editor {
        app: app.clone(),
        id,
        model: Rc::new(RefCell::new(model)),
        dialog: dialog.clone(),
        banner: adw::Banner::new(""),
    });
    editor.build();
    dialog.present(Some(&app.parent()));
}

fn enum_value<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn parse_enum<T: DeserializeOwned>(value: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(value.into())).ok()
}

fn group(title: &str) -> adw::PreferencesGroup {
    adw::PreferencesGroup::builder().title(title).build()
}

impl Editor {
    fn build(self: &Rc<Self>) {
        let stack = adw::ViewStack::new();
        let pages = [
            ("general", "General", "preferences-system-symbolic", self.general()),
            ("auth", "Auth", "dialog-password-symbolic", self.authentication()),
            ("options", "Options", "emblem-system-symbolic", self.options()),
            ("network", "Network", "network-workgroup-symbolic", self.networking()),
            ("proxy", "Proxy", "network-server-symbolic", self.proxy()),
            (
                "advanced",
                "Advanced",
                "applications-engineering-symbolic",
                self.advanced(),
            ),
        ];
        for (name, title, icon, page) in pages {
            stack.add_titled_with_icon(&page, Some(name), title, icon);
        }
        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .halign(gtk::Align::Center)
            .margin_bottom(6)
            .build();
        let header = adw::HeaderBar::builder()
            .show_end_title_buttons(false)
            .show_start_title_buttons(false)
            .build();
        let cancel = gtk::Button::with_label("Cancel");
        let dialog = self.dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
        let save = gtk::Button::builder()
            .label("Save")
            .css_classes(["suggested-action"])
            .build();
        let this = self.clone();
        save.connect_clicked(move |_| this.save());
        header.pack_start(&cancel);
        header.pack_end(&save);
        let bottom = adw::ViewSwitcherBar::builder().stack(&stack).build();
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.add_top_bar(&switcher);
        self.banner.set_revealed(false);
        view.add_top_bar(&self.banner);
        view.set_content(Some(&stack));
        view.add_bottom_bar(&bottom);
        self.dialog.set_child(Some(&view));
        self.dialog.set_default_widget(Some(&save));
        let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 760sp").expect("condition"));
        breakpoint.add_setter(&bottom, "reveal", Some(&true.to_value()));
        breakpoint.add_setter(&switcher, "visible", Some(&false.to_value()));
        self.dialog.add_breakpoint(breakpoint);
        if let Some(id) = &self.id
            && self.app.is_active(id)
        {
            self.banner.set_title("Changes apply the next time you connect.");
            self.banner.set_revealed(true);
        }
    }

    fn save(&self) {
        let model = self.model.borrow().clone();
        let problems = model.validate();
        if !problems.is_empty() {
            self.show_problems(&problems.join("\n"));
            return;
        }
        match fluidity_core::save_model(&self.app.store, self.id.as_deref(), &model) {
            Ok(id) => {
                self.app.reload();
                self.app.window().select(&id);
                self.dialog.close();
            }
            Err(e) => self.show_problems(&e),
        }
    }

    fn show_problems(&self, text: &str) {
        let alert = adw::AlertDialog::new(Some("Cannot Save Connection"), Some(text));
        alert.add_response("ok", "OK");
        alert.present(Some(&self.dialog));
    }

    fn entry(&self, title: &str, get: fn(&EditorModel) -> String, set: fn(&mut EditorModel, String)) -> adw::EntryRow {
        let row = adw::EntryRow::builder()
            .title(title)
            .text(get(&self.model.borrow()))
            .build();
        let model = self.model.clone();
        row.connect_changed(move |r| set(&mut model.borrow_mut(), r.text().to_string()));
        row
    }

    fn number<T>(
        &self,
        title: &str,
        get: fn(&EditorModel) -> Option<T>,
        set: fn(&mut EditorModel, Option<T>),
    ) -> adw::EntryRow
    where
        T: std::str::FromStr + ToString + 'static,
    {
        let row = adw::EntryRow::builder()
            .title(title)
            .input_purpose(gtk::InputPurpose::Digits)
            .text(get(&self.model.borrow()).map(|v| v.to_string()).unwrap_or_default())
            .build();
        let model = self.model.clone();
        row.connect_changed(move |r| {
            let text = r.text();
            let value = text.trim().parse().ok();
            if value.is_none() && !text.trim().is_empty() {
                r.add_css_class("error");
            } else {
                r.remove_css_class("error");
            }
            set(&mut model.borrow_mut(), value);
        });
        row
    }

    fn switch(&self, title: &str, get: fn(&EditorModel) -> bool, set: fn(&mut EditorModel, bool)) -> adw::SwitchRow {
        let row = adw::SwitchRow::builder()
            .title(title)
            .active(get(&self.model.borrow()))
            .build();
        let model = self.model.clone();
        row.connect_active_notify(move |r| set(&mut model.borrow_mut(), r.is_active()));
        row
    }

    fn choice(
        &self,
        title: &str,
        options: &[(&str, &str)],
        get: fn(&EditorModel) -> String,
        set: fn(&mut EditorModel, &str),
    ) -> adw::ComboRow {
        let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
        let values: Vec<String> = options.iter().map(|o| o.0.to_string()).collect();
        let current = get(&self.model.borrow());
        let row = adw::ComboRow::builder()
            .title(title)
            .model(&gtk::StringList::new(&labels))
            .selected(values.iter().position(|v| *v == current).unwrap_or(0) as u32)
            .build();
        let model = self.model.clone();
        row.connect_selected_notify(move |r| {
            if let Some(v) = values.get(r.selected() as usize) {
                set(&mut model.borrow_mut(), v);
            }
        });
        row
    }

    fn list(
        &self,
        title: &str,
        get: fn(&EditorModel) -> &Vec<String>,
        set: fn(&mut EditorModel, Vec<String>),
    ) -> adw::EntryRow {
        let row = adw::EntryRow::builder()
            .title(title)
            .text(get(&self.model.borrow()).join(", "))
            .build();
        let model = self.model.clone();
        row.connect_changed(move |r| {
            let values = r
                .text()
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            set(&mut model.borrow_mut(), values);
        });
        row
    }

    fn file(&self, title: &str, field: fn(&mut EditorModel) -> &mut FileField) -> adw::ActionRow {
        let row = adw::ActionRow::builder().title(title).use_markup(false).build();
        let describe = |f: &FileField| {
            if !f.is_set() {
                "None".to_string()
            } else if f.name.is_empty() {
                "Embedded in the configuration".to_string()
            } else {
                f.name.clone()
            }
        };
        row.set_subtitle(&describe(field(&mut self.model.borrow_mut())));
        let clear = gtk::Button::builder()
            .icon_name("edit-clear-symbolic")
            .tooltip_text("Remove")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .visible(field(&mut self.model.borrow_mut()).is_set())
            .build();
        let choose = gtk::Button::builder()
            .label("Choose…")
            .valign(gtk::Align::Center)
            .build();
        row.add_suffix(&clear);
        row.add_suffix(&choose);
        let model = self.model.clone();
        let (r, c) = (row.clone(), clear.clone());
        clear.connect_clicked(move |_| {
            *field(&mut model.borrow_mut()) = FileField::default();
            r.set_subtitle("None");
            c.set_visible(false);
        });
        let model = self.model.clone();
        let dialog = self.dialog.clone();
        let title = title.to_string();
        let row_ref = row.clone();
        choose.connect_clicked(move |_| {
            let row = row_ref.clone();
            let picker = gtk::FileDialog::builder().title(format!("Choose {title}")).build();
            let (model, clear) = (model.clone(), clear.clone());
            let parent = dialog.root().and_downcast::<gtk::Window>();
            glib::spawn_future_local(async move {
                let Ok(file) = picker.open_future(parent.as_ref()).await else {
                    return;
                };
                let Some(path) = file.path() else { return };
                let Ok(data) = std::fs::read(&path) else { return };
                let value = FileField {
                    name: path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    data: Some(STANDARD.encode(data)),
                };
                row.set_subtitle(&value.name);
                *field(&mut model.borrow_mut()) = value;
                clear.set_visible(true);
            });
        });
        row
    }

    fn script(&self, title: &str, field: fn(&mut EditorModel) -> &mut String) -> adw::ActionRow {
        let row = adw::ActionRow::builder().title(title).use_markup(false).build();
        let show = |r: &adw::ActionRow, v: &str| {
            let name = std::path::Path::new(v)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            r.set_subtitle(if v.is_empty() { "None" } else { &name });
        };
        show(&row, field(&mut self.model.borrow_mut()));
        let clear = gtk::Button::builder()
            .icon_name("edit-clear-symbolic")
            .tooltip_text("Remove")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let choose = gtk::Button::builder()
            .label("Choose…")
            .valign(gtk::Align::Center)
            .build();
        row.add_suffix(&clear);
        row.add_suffix(&choose);
        let model = self.model.clone();
        let r = row.clone();
        clear.connect_clicked(move |_| {
            field(&mut model.borrow_mut()).clear();
            show(&r, "");
        });
        let model = self.model.clone();
        let dialog = self.dialog.clone();
        let row_ref = row.clone();
        choose.connect_clicked(move |_| {
            let picker = gtk::FileDialog::builder().title("Choose Script").build();
            let (model, row) = (model.clone(), row_ref.clone());
            let parent = dialog.root().and_downcast::<gtk::Window>();
            glib::spawn_future_local(async move {
                let Ok(file) = picker.open_future(parent.as_ref()).await else {
                    return;
                };
                let Some(path) = file.path() else { return };
                let value = path.to_string_lossy().into_owned();
                show(&row, &value);
                *field(&mut model.borrow_mut()) = value;
            });
        });
        row
    }

    fn general(self: &Rc<Self>) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();
        let g = adw::PreferencesGroup::new();
        g.add(&self.entry("Name", |m| m.name.clone(), |m, v| m.name = v));
        page.add(&g);

        let servers = group("Servers");
        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Add Server")
            .css_classes(["flat"])
            .build();
        servers.set_header_suffix(Some(&add));
        let rows: Rc<RefCell<Vec<adw::ExpanderRow>>> = Rc::default();
        let this = self.clone();
        let servers_ref = servers.clone();
        let rebuild: Rc<dyn Fn()> = Rc::new({
            let rows = rows.clone();
            move || this.rebuild_remotes(&servers_ref, &rows)
        });
        rebuild();
        let model = self.model.clone();
        let r = rebuild.clone();
        add.connect_clicked(move |_| {
            model.borrow_mut().remotes.push(Remote {
                port: 1194,
                ..Remote::default()
            });
            r();
        });
        page.add(&servers);

        let behavior = adw::PreferencesGroup::new();
        behavior.add(&self.switch(
            "Try Servers in Random Order",
            |m| m.remote_random,
            |m, v| m.remote_random = v,
        ));
        behavior.add(&self.number(
            "Server Timeout (seconds)",
            |m| m.server_timeout,
            |m, v| m.server_timeout = v,
        ));
        page.add(&behavior);

        let device = group("Interface");
        let dhcp = self.switch("Use DHCP on the TAP Device", |m| m.dhcp, |m, v| m.dhcp = v);
        let combo = self.choice(
            "Device",
            &[("tun", "TUN (routed)"), ("tap", "TAP (bridged)")],
            |m| enum_value(&m.device),
            |m, v| {
                if let Some(d) = parse_enum(v) {
                    m.device = d
                }
            },
        );
        let d = dhcp.clone();
        let model = self.model.clone();
        let sync = move || d.set_sensitive(enum_value(&model.borrow().device) == "tap");
        sync();
        combo.connect_selected_notify(move |_| sync());
        device.add(&combo);
        device.add(&dhcp);
        device.add(&self.switch("Enable IPv6", |m| m.ipv6, |m, v| m.ipv6 = v));
        device.add(&self.switch(
            "Reconnect Automatically",
            |m| m.auto_reconnect,
            |m, v| m.auto_reconnect = v,
        ));
        device.add(&self.switch(
            "Connect When Fluidity Starts",
            |m| m.connect_on_launch,
            |m, v| m.connect_on_launch = v,
        ));
        page.add(&device);
        page
    }

    fn rebuild_remotes(self: &Rc<Self>, group: &adw::PreferencesGroup, rows: &Rc<RefCell<Vec<adw::ExpanderRow>>>) {
        for row in rows.borrow_mut().drain(..) {
            group.remove(&row);
        }
        let count = self.model.borrow().remotes.len();
        for index in 0..count {
            let remote = self.model.borrow().remotes[index].clone();
            let summary = |r: &Remote| {
                if r.host.is_empty() {
                    "New Server".to_string()
                } else {
                    format!("{}:{}", r.host, r.port)
                }
            };
            let expander = adw::ExpanderRow::builder()
                .title(summary(&remote))
                .subtitle(enum_value(&remote.transport).to_uppercase())
                .expanded(remote.host.is_empty())
                .build();
            let host = adw::EntryRow::builder().title("Address").text(&remote.host).build();
            let port = adw::EntryRow::builder()
                .title("Port")
                .input_purpose(gtk::InputPurpose::Digits)
                .text(remote.port.to_string())
                .build();
            let transport = adw::ComboRow::builder()
                .title("Protocol")
                .model(&gtk::StringList::new(&["UDP", "TCP"]))
                .selected(if enum_value(&remote.transport) == "tcp" { 1 } else { 0 })
                .build();
            let versions = ["automatic", "ipv4", "ipv6"];
            let ip = adw::ComboRow::builder()
                .title("IP Version")
                .model(&gtk::StringList::new(&["Automatic", "IPv4", "IPv6"]))
                .selected(
                    versions
                        .iter()
                        .position(|v| *v == enum_value(&remote.ip_version))
                        .unwrap_or(0) as u32,
                )
                .build();
            let model = self.model.clone();
            let e = expander.clone();
            let refresh = move |m: &EditorModel| {
                if let Some(r) = m.remotes.get(index) {
                    e.set_title(&summary(r));
                    e.set_subtitle(&enum_value(&r.transport).to_uppercase());
                }
            };
            let refresh = Rc::new(refresh);
            {
                let (model, refresh) = (model.clone(), refresh.clone());
                host.connect_changed(move |r| {
                    let mut m = model.borrow_mut();
                    if let Some(remote) = m.remotes.get_mut(index) {
                        remote.host = r.text().trim().to_string();
                    }
                    refresh(&m);
                });
            }
            {
                let (model, refresh) = (model.clone(), refresh.clone());
                port.connect_changed(move |r| {
                    let mut m = model.borrow_mut();
                    if let Some(remote) = m.remotes.get_mut(index) {
                        remote.port = r.text().trim().parse().unwrap_or(0);
                    }
                    refresh(&m);
                });
            }
            {
                let (model, refresh) = (model.clone(), refresh.clone());
                transport.connect_selected_notify(move |r| {
                    let mut m = model.borrow_mut();
                    if let Some(remote) = m.remotes.get_mut(index)
                        && let Some(t) = parse_enum(if r.selected() == 1 { "tcp" } else { "udp" })
                    {
                        remote.transport = t;
                    }
                    refresh(&m);
                });
            }
            {
                let model = model.clone();
                ip.connect_selected_notify(move |r| {
                    let mut m = model.borrow_mut();
                    if let Some(remote) = m.remotes.get_mut(index)
                        && let Some(v) = parse_enum(versions[r.selected() as usize % 3])
                    {
                        remote.ip_version = v;
                    }
                });
            }
            expander.add_row(&host);
            expander.add_row(&port);
            expander.add_row(&transport);
            expander.add_row(&ip);
            if count > 1 {
                let remove = gtk::Button::builder()
                    .icon_name("user-trash-symbolic")
                    .tooltip_text("Remove Server")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                let this = self.clone();
                let (group, rows) = (group.clone(), rows.clone());
                remove.connect_clicked(move |_| {
                    this.model.borrow_mut().remotes.remove(index);
                    let this2 = this.clone();
                    let (group, rows) = (group.clone(), rows.clone());
                    glib::idle_add_local_once(move || this2.rebuild_remotes(&group, &rows));
                });
                expander.add_suffix(&remove);
            }
            group.add(&expander);
            rows.borrow_mut().push(expander);
        }
    }

    fn authentication(self: &Rc<Self>) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();
        let main = adw::PreferencesGroup::new();
        let auth = self.choice(
            "Type",
            &[
                ("tls", "SSL/TLS Client"),
                ("pkcs12", "PKCS#12 File"),
                ("pkcs11", "PKCS#11 Token"),
                ("static_key", "Static Key"),
                ("none", "None"),
            ],
            |m| enum_value(&m.auth_type),
            |m, v| {
                if let Some(t) = parse_enum(v) {
                    m.auth_type = t
                }
            },
        );
        main.add(&auth);
        main.add(&self.switch("Require Username and Password", |m| m.user_pass, |m, v| m.user_pass = v));
        page.add(&main);

        let certs = group("Certificates");
        let ca = self.file("CA Certificate", |m| &mut m.ca);
        let cert = self.file("Certificate", |m| &mut m.cert);
        let key = self.file("Private Key", |m| &mut m.key);
        let pkcs12 = self.file("PKCS#12 File", |m| &mut m.pkcs12);
        let providers = self.entry(
            "Provider Library",
            |m| m.pkcs11_providers.clone(),
            |m, v| m.pkcs11_providers = v,
        );
        let token = self.entry("Token ID", |m| m.pkcs11_id.clone(), |m, v| m.pkcs11_id = v);
        let token_prompt = self.switch(
            "Choose the Token When Connecting",
            |m| m.pkcs11_prompt,
            |m, v| m.pkcs11_prompt = v,
        );
        let static_key = self.file("Static Key", |m| &mut m.static_key);
        let static_dir = self.direction("Key Direction", |m| &mut m.static_key_direction);
        for w in [
            ca.upcast_ref::<gtk::Widget>(),
            cert.upcast_ref(),
            key.upcast_ref(),
            pkcs12.upcast_ref(),
            providers.upcast_ref(),
            token.upcast_ref(),
            token_prompt.upcast_ref(),
            static_key.upcast_ref(),
            static_dir.upcast_ref(),
        ] {
            certs.add(w);
        }
        page.add(&certs);

        let tls = group("Control Channel");
        let extra_key = self.file("Key", |m| &mut m.extra_key);
        let extra_dir = self.direction("Key Direction", |m| &mut m.extra_direction);
        let extra = self.choice(
            "Protection",
            &[
                ("none", "None"),
                ("tls_auth", "TLS Auth"),
                ("tls_crypt", "TLS Crypt"),
                ("tls_crypt_v2", "TLS Crypt v2"),
            ],
            |m| enum_value(&m.extra_type),
            |m, v| {
                if let Some(t) = parse_enum(v) {
                    m.extra_type = t
                }
            },
        );
        tls.add(&extra);
        tls.add(&extra_key);
        tls.add(&extra_dir);
        page.add(&tls);

        let model = self.model.clone();
        let tls_group = tls.clone();
        let sync = Rc::new(move || {
            let m = model.borrow();
            let kind = enum_value(&m.auth_type);
            let is = |k: &[&str]| k.contains(&kind.as_str());
            ca.set_visible(is(&["tls", "pkcs12", "pkcs11"]));
            cert.set_visible(is(&["tls"]));
            key.set_visible(is(&["tls"]));
            pkcs12.set_visible(is(&["pkcs12"]));
            providers.set_visible(is(&["pkcs11"]));
            token.set_visible(is(&["pkcs11"]));
            token_prompt.set_visible(is(&["pkcs11"]));
            static_key.set_visible(is(&["static_key"]));
            static_dir.set_visible(is(&["static_key"]));
            certs.set_visible(!is(&["none"]));
            tls_group.set_visible(!is(&["static_key"]));
            let extra = enum_value(&m.extra_type);
            extra_key.set_visible(extra != "none");
            extra_dir.set_visible(extra == "tls_auth");
        });
        sync();
        let s = sync.clone();
        auth.connect_selected_notify(move |_| s());
        extra.connect_selected_notify(move |_| sync());
        page
    }

    fn direction(&self, title: &str, field: fn(&mut EditorModel) -> &mut Option<u8>) -> adw::ComboRow {
        let current = *field(&mut self.model.borrow_mut());
        let row = adw::ComboRow::builder()
            .title(title)
            .model(&gtk::StringList::new(&["None", "0", "1"]))
            .selected(current.map(|d| d as u32 + 1).unwrap_or(0))
            .build();
        let model = self.model.clone();
        row.connect_selected_notify(move |r| {
            *field(&mut model.borrow_mut()) = r.selected().checked_sub(1).map(|d| d as u8);
        });
        row
    }

    fn options(self: &Rc<Self>) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();
        let keepalive = group("Keepalive");
        keepalive.add(&self.number("Ping Interval (seconds)", |m| m.ping, |m, v| m.ping = v));
        keepalive.add(&self.number("Restart After (seconds)", |m| m.ping_restart, |m, v| m.ping_restart = v));
        page.add(&keepalive);
        let persist = group("Persistence");
        persist.add(&self.switch("Persist TUN Device", |m| m.persist_tun, |m, v| m.persist_tun = v));
        persist.add(&self.switch("Persist Key", |m| m.persist_key, |m, v| m.persist_key = v));
        persist.add(&self.switch(
            "Persist Local IP",
            |m| m.persist_local_ip,
            |m, v| m.persist_local_ip = v,
        ));
        persist.add(&self.switch(
            "Persist Remote IP",
            |m| m.persist_remote_ip,
            |m, v| m.persist_remote_ip = v,
        ));
        page.add(&persist);
        let behavior = group("Behavior");
        behavior.add(&self.switch(
            "Verify Server Certificate Usage",
            |m| m.remote_cert_tls,
            |m, v| m.remote_cert_tls = v,
        ));
        behavior.add(&self.switch("Do Not Bind a Local Port", |m| m.nobind, |m, v| m.nobind = v));
        behavior.add(&self.switch("Pull Options From the Server", |m| m.pull, |m, v| m.pull = v));
        behavior.add(&self.choice(
            "Compression",
            &[
                ("off", "Off"),
                ("none", "None (framing only)"),
                ("lzo", "LZO"),
                ("lz4", "LZ4"),
                ("lz4_v2", "LZ4 v2"),
                ("stub", "Stub"),
                ("stub_v2", "Stub v2"),
                ("adaptive", "Adaptive LZO"),
            ],
            |m| enum_value(&m.compression),
            |m, v| {
                if let Some(c) = parse_enum(v) {
                    m.compression = c
                }
            },
        ));
        behavior.add(&self.choice(
            "Compatibility",
            &[
                ("", "Current"),
                ("2.6", "OpenVPN 2.6"),
                ("2.5", "OpenVPN 2.5"),
                ("2.4", "OpenVPN 2.4"),
                ("2.3", "OpenVPN 2.3"),
            ],
            |m| m.compatibility.clone(),
            |m, v| m.compatibility = v.into(),
        ));
        page.add(&behavior);
        page
    }

    fn networking(self: &Rc<Self>) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();
        let traffic = group("Traffic");
        traffic.add(&self.choice(
            "Send Traffic Over VPN",
            &[
                ("automatic", "As the Server Sets"),
                ("all", "All Traffic"),
                ("ipv4", "All IPv4 Traffic"),
                ("ipv6", "All IPv6 Traffic"),
            ],
            |m| enum_value(&m.redirect),
            |m, v| {
                if let Some(r) = parse_enum(v) {
                    m.redirect = r
                }
            },
        ));
        traffic.add(&self.entry(
            "Default Gateway",
            |m| m.default_gateway.clone(),
            |m, v| m.default_gateway = v,
        ));
        page.add(&traffic);

        let routes = group("Routes");
        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Add Route")
            .css_classes(["flat"])
            .build();
        routes.set_header_suffix(Some(&add));
        let rows: Rc<RefCell<Vec<adw::ExpanderRow>>> = Rc::default();
        self.rebuild_routes(&routes, &rows);
        let this = self.clone();
        let (g, r) = (routes.clone(), rows.clone());
        add.connect_clicked(move |_| {
            this.model.borrow_mut().routes.push(Route::default());
            this.rebuild_routes(&g, &r);
        });
        page.add(&routes);

        let dns = group("DNS");
        dns.add(&self.choice(
            "Mode",
            &[
                ("automatic", "Automatic"),
                ("full", "Full (all queries)"),
                ("split", "Split (listed domains)"),
                ("off", "Off"),
            ],
            |m| m.dns_mode.clone(),
            |m, v| m.dns_mode = v.into(),
        ));
        dns.add(&self.list("Servers", |m| &m.dns_servers, |m, v| m.dns_servers = v));
        dns.add(&self.list("Domains", |m| &m.dns_domains, |m, v| m.dns_domains = v));
        dns.add(&self.switch(
            "Ignore DNS Settings From the Server",
            |m| m.ignore_pushed_dns,
            |m, v| m.ignore_pushed_dns = v,
        ));
        page.add(&dns);

        let link = group("Link");
        link.add(&self.number("Traffic Shaping (bytes/s)", |m| m.shaper, |m, v| m.shaper = v));
        link.add(&self.number("Fragment Size", |m| m.fragment, |m, v| m.fragment = v));
        link.add(&self.number("TUN MTU", |m| m.tun_mtu, |m, v| m.tun_mtu = v));
        link.add(&self.number(
            "Disconnect When Inactive (seconds)",
            |m| m.inactive,
            |m, v| m.inactive = v,
        ));
        page.add(&link);
        page
    }

    fn rebuild_routes(self: &Rc<Self>, group: &adw::PreferencesGroup, rows: &Rc<RefCell<Vec<adw::ExpanderRow>>>) {
        for row in rows.borrow_mut().drain(..) {
            group.remove(&row);
        }
        let count = self.model.borrow().routes.len();
        for index in 0..count {
            let route = self.model.borrow().routes[index].clone();
            let title = |r: &Route| {
                if r.network.is_empty() {
                    "New Route".to_string()
                } else if r.mask.is_empty() {
                    r.network.clone()
                } else {
                    format!("{} / {}", r.network, r.mask)
                }
            };
            let expander = adw::ExpanderRow::builder()
                .title(title(&route))
                .expanded(route.network.is_empty())
                .build();
            let fields: [(&str, String, RouteField); 4] = [
                ("Network", route.network.clone(), |r| &mut r.network),
                ("Mask or Prefix", route.mask.clone(), |r| &mut r.mask),
                ("Gateway", route.gateway.clone(), |r| &mut r.gateway),
                ("Metric", route.metric.clone(), |r| &mut r.metric),
            ];
            for (label, value, field) in fields {
                let entry = adw::EntryRow::builder().title(label).text(&value).build();
                let model = self.model.clone();
                let e = expander.clone();
                entry.connect_changed(move |r| {
                    let mut m = model.borrow_mut();
                    if let Some(route) = m.routes.get_mut(index) {
                        *field(route) = r.text().trim().to_string();
                        e.set_title(&title(route));
                    }
                });
                expander.add_row(&entry);
            }
            let ipv6 = adw::SwitchRow::builder().title("IPv6").active(route.ipv6).build();
            let model = self.model.clone();
            ipv6.connect_active_notify(move |r| {
                if let Some(route) = model.borrow_mut().routes.get_mut(index) {
                    route.ipv6 = r.is_active();
                }
            });
            expander.add_row(&ipv6);
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text("Remove Route")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let this = self.clone();
            let (g, r) = (group.clone(), rows.clone());
            remove.connect_clicked(move |_| {
                this.model.borrow_mut().routes.remove(index);
                let this2 = this.clone();
                let (g, r) = (g.clone(), r.clone());
                glib::idle_add_local_once(move || this2.rebuild_routes(&g, &r));
            });
            expander.add_suffix(&remove);
            group.add(&expander);
            rows.borrow_mut().push(expander);
        }
    }

    fn proxy(self: &Rc<Self>) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();
        let proxy = group("Proxy");
        let kind = self.choice(
            "Type",
            &[
                ("none", "None"),
                ("http", "HTTP"),
                ("socks", "SOCKS"),
                ("system", "System Proxy"),
            ],
            |m| enum_value(&m.proxy_type),
            |m, v| {
                if let Some(t) = parse_enum(v) {
                    m.proxy_type = t
                }
            },
        );
        let host = self.entry("Address", |m| m.proxy_host.clone(), |m, v| m.proxy_host = v);
        let port = self.number("Port", |m| m.proxy_port, |m, v| m.proxy_port = v);
        let auth = self.choice(
            "Authentication",
            &[
                ("none", "None"),
                ("basic", "Basic"),
                ("ntlm", "NTLM"),
                ("automatic", "Automatic"),
            ],
            |m| enum_value(&m.proxy_auth),
            |m, v| {
                if let Some(a) = parse_enum(v) {
                    m.proxy_auth = a
                }
            },
        );
        proxy.add(&kind);
        proxy.add(&host);
        proxy.add(&port);
        proxy.add(&auth);
        page.add(&proxy);

        let obfs = group("Obfuscation");
        obfs.set_description(Some("Disguise OpenVPN traffic with a pluggable transport."));
        let method = self.choice(
            "Method",
            &[
                ("", "None"),
                ("obfs2", "obfs2"),
                ("obfs3", "obfs3"),
                ("obfs4", "obfs4"),
                ("scramblesuit", "ScrambleSuit"),
            ],
            |m| m.obfuscation.clone(),
            |m, v| m.obfuscation = v.into(),
        );
        let key = self.entry(
            "Key or Certificate",
            |m| m.obfuscation_key.clone(),
            |m, v| m.obfuscation_key = v,
        );
        obfs.add(&method);
        obfs.add(&key);
        page.add(&obfs);

        let model = self.model.clone();
        let sync = Rc::new(move || {
            let m = model.borrow();
            let t = enum_value(&m.proxy_type);
            host.set_visible(t == "http" || t == "socks");
            port.set_visible(t == "http" || t == "socks");
            auth.set_visible(t == "http");
            key.set_visible(m.obfuscation == "obfs4" || m.obfuscation == "scramblesuit");
        });
        sync();
        let s = sync.clone();
        kind.connect_selected_notify(move |_| s());
        method.connect_selected_notify(move |_| sync());
        page
    }

    fn advanced(self: &Rc<Self>) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();
        let scripts = group("Scripts");
        scripts.set_description(Some(
            "Scripts run as you. Before-connect output can set FluidityUsername:, FluidityPassword: or FluidityNoConnect.",
        ));
        scripts.add(&self.script("Before Connect", |m| &mut m.before_connect_script));
        scripts.add(&self.script("Connected", |m| &mut m.connected_script));
        scripts.add(&self.script("Disconnected", |m| &mut m.disconnected_script));
        page.add(&scripts);

        let commands = group("Additional OpenVPN Commands");
        commands.set_description(Some(
            "One command per line. Lines that start with #fluidity set Fluidity options.",
        ));
        let view = gtk::TextView::builder()
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(10)
            .bottom_margin(10)
            .left_margin(10)
            .right_margin(10)
            .build();
        view.buffer().set_text(&self.model.borrow().extra_commands);
        let model = self.model.clone();
        view.buffer().connect_changed(move |b| {
            model.borrow_mut().extra_commands = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
        });
        let scroll = gtk::ScrolledWindow::builder()
            .min_content_height(200)
            .child(&view)
            .css_classes(["card"])
            .build();
        commands.add(&scroll);
        page.add(&commands);
        page
    }
}
