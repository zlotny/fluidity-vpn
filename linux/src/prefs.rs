use std::cell::RefCell;
#[cfg(not(windows))]
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use fluidity_core::{AddressDisplay, Folder, GraphStyle, Node, Preferences, TrafficDisplay};
use gtk::glib;

use crate::app::App;
use crate::state;

pub const APP_ID: &str = "org.fluidityvpn.Fluidity";

#[cfg(not(windows))]
fn autostart_file() -> PathBuf {
    glib::user_config_dir()
        .join("autostart")
        .join(format!("{APP_ID}.desktop"))
}

#[cfg(not(windows))]
pub fn set_autostart(enabled: bool) -> std::io::Result<()> {
    let path = autostart_file();
    if !enabled {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "fluidity-gtk".into());
    std::fs::create_dir_all(path.parent().expect("parent"))?;
    std::fs::write(
        &path,
        format!(
            "[Desktop Entry]\nType=Application\nName=Fluidity\nExec={exe} --hidden\nIcon={APP_ID}\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n"
        ),
    )
}

#[cfg(windows)]
pub fn set_autostart(enabled: bool) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("reg");
    if enabled {
        let exe = std::env::current_exe()?;
        let value = format!("\"{}\" --hidden", exe.display());
        command.args(["add", RUN_KEY, "/v", "Fluidity", "/t", "REG_SZ", "/d", &value, "/f"]);
    } else {
        command.args(["delete", RUN_KEY, "/v", "Fluidity", "/f"]);
    }
    let status = command.creation_flags(CREATE_NO_WINDOW).status()?;
    if status.success() || !enabled {
        Ok(())
    } else {
        Err(std::io::Error::other("reg failed"))
    }
}

fn switch(
    app: &Rc<App>,
    title: &str,
    get: fn(&Preferences) -> bool,
    set: fn(&mut Preferences, bool),
) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title(title)
        .active(get(&app.prefs.borrow()))
        .build();
    let app = app.clone();
    row.connect_active_notify(move |r| {
        set(&mut app.prefs.borrow_mut(), r.is_active());
        app.save_prefs();
    });
    row
}

fn choice(
    app: &Rc<App>,
    title: &str,
    options: &[&str],
    get: fn(&Preferences) -> usize,
    set: fn(&mut Preferences, usize),
) -> adw::ComboRow {
    let row = adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(options))
        .selected(get(&app.prefs.borrow()) as u32)
        .build();
    let app = app.clone();
    row.connect_selected_notify(move |r| {
        set(&mut app.prefs.borrow_mut(), r.selected() as usize);
        app.save_prefs();
    });
    row
}

fn wifi_groups(app: &Rc<App>, page: &adw::PreferencesPage) {
    app.check_wifi();
    let wifi = adw::PreferencesGroup::builder().title("Wi-Fi").build();
    let connections = app.connections.borrow().clone();
    let mut names = vec!["Off"];
    names.extend(connections.iter().map(|c| c.name.as_str()));
    let current = app.prefs.borrow().auto_connect.clone();
    let selected = current
        .and_then(|id| connections.iter().position(|c| c.id == id))
        .map(|i| i + 1)
        .unwrap_or(0);
    let auto = adw::ComboRow::builder()
        .title("Connect Automatically on Untrusted Wi-Fi")
        .model(&gtk::StringList::new(&names))
        .selected(selected as u32)
        .build();
    let disconnect = switch(
        app,
        "Disconnect on Trusted Networks",
        |p| p.disconnect_on_trusted,
        |p, v| p.disconnect_on_trusted = v,
    );
    wifi.add(&auto);
    wifi.add(&disconnect);
    page.add(&wifi);

    let trusted = adw::PreferencesGroup::builder()
        .title("Trusted Networks")
        .description("Fluidity connects automatically when you join any other Wi-Fi network.")
        .build();
    let rows: Rc<RefCell<Vec<gtk::Widget>>> = Rc::default();
    let entry = adw::EntryRow::builder()
        .title("Add Network")
        .show_apply_button(true)
        .build();
    let current_row = adw::ActionRow::new();
    let trust = gtk::Button::builder().label("Trust").valign(gtk::Align::Center).build();
    current_row.add_suffix(&trust);
    trusted.add(&entry);
    trusted.add(&current_row);
    page.add(&trusted);

    let refresh: Rc<dyn Fn()> = {
        let app = app.clone();
        let trusted = trusted.clone();
        let rows = rows.clone();
        let current_row = current_row.clone();
        Rc::new(move || {
            for row in rows.borrow_mut().drain(..) {
                trusted.remove(&row);
            }
            let list = app.prefs.borrow().trusted_networks.clone();
            for name in list {
                let row = adw::ActionRow::builder().title(glib::markup_escape_text(&name)).build();
                let remove = gtk::Button::builder()
                    .icon_name("list-remove-symbolic")
                    .tooltip_text("Remove")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                let app = app.clone();
                remove.connect_clicked(move |button| {
                    app.prefs.borrow_mut().trusted_networks.retain(|n| *n != name);
                    app.save_prefs();
                    if let Some(row) = button.ancestor(adw::ActionRow::static_type()) {
                        row.set_visible(false);
                    }
                });
                row.add_suffix(&remove);
                trusted.add(&row);
                rows.borrow_mut().push(row.upcast());
            }
            let ssid = app.ssid.borrow().clone();
            let show = ssid
                .as_ref()
                .is_some_and(|s| !app.prefs.borrow().trusted_networks.contains(s));
            current_row.set_visible(show);
            if let Some(ssid) = ssid {
                current_row.set_title(&glib::markup_escape_text(&format!("Current Network: {ssid}")));
            }
        })
    };
    refresh();
    {
        let refresh = refresh.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || refresh());
    }

    let add: Rc<dyn Fn(String)> = {
        let app = app.clone();
        let refresh = refresh.clone();
        Rc::new(move |name: String| {
            let name = name.trim().to_string();
            if name.is_empty() || app.prefs.borrow().is_trusted(&name) {
                return;
            }
            app.prefs.borrow_mut().trusted_networks.push(name);
            app.save_prefs();
            refresh();
        })
    };
    {
        let add = add.clone();
        entry.connect_apply(move |e| {
            add(e.text().to_string());
            e.set_text("");
        });
    }
    {
        let app = app.clone();
        trust.connect_clicked(move |_| {
            let ssid = app.ssid.borrow().clone();
            if let Some(ssid) = ssid {
                add(ssid);
            }
        });
    }

    let set_visible = {
        let disconnect = disconnect.clone();
        let trusted = trusted.clone();
        move |on: bool| {
            disconnect.set_visible(on);
            trusted.set_visible(on);
        }
    };
    set_visible(selected != 0);
    let app = app.clone();
    auto.connect_selected_notify(move |row| {
        let index = row.selected() as usize;
        let id = index
            .checked_sub(1)
            .and_then(|i| connections.get(i))
            .map(|c| c.id.clone());
        let on = id.is_some();
        app.prefs.borrow_mut().auto_connect = id;
        app.save_prefs();
        set_visible(on);
        if on {
            app.check_wifi();
        }
        refresh();
    });
}

const IDLE: [Option<u32>; 6] = [None, Some(5), Some(15), Some(30), Some(60), Some(120)];

pub fn open(app: &Rc<App>) {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_search_enabled(false);

    let general = adw::PreferencesPage::builder()
        .title("General")
        .icon_name("preferences-system-symbolic")
        .build();
    let startup = adw::PreferencesGroup::new();
    let login = switch(
        app,
        "Start Fluidity at Login",
        |p| p.start_at_login,
        |p, v| p.start_at_login = v,
    );
    let a = app.clone();
    login.connect_active_notify(move |r| {
        if let Err(e) = set_autostart(r.is_active()) {
            a.error(&format!("Cannot change the login item: {e}"));
        }
    });
    startup.add(&login);
    startup.add(&switch(
        app,
        "Show Notifications",
        |p| p.notifications,
        |p, v| p.notifications = v,
    ));
    general.add(&startup);

    let reconnect = adw::PreferencesGroup::builder().title("Reconnect").build();
    reconnect.add(&switch(
        app,
        "Reconnect After Waking From Sleep",
        |p| p.reconnect_on_wake,
        |p, v| p.reconnect_on_wake = v,
    ));
    reconnect.add(&switch(
        app,
        "Reconnect When the Network Returns",
        |p| p.reconnect_on_network,
        |p, v| p.reconnect_on_network = v,
    ));
    reconnect.add(&choice(
        app,
        "Disconnect When Idle",
        &[
            "Never",
            "After 5 Minutes",
            "After 15 Minutes",
            "After 30 Minutes",
            "After 1 Hour",
            "After 2 Hours",
        ],
        |p| IDLE.iter().position(|v| *v == p.idle_disconnect_minutes).unwrap_or(0),
        |p, i| p.idle_disconnect_minutes = IDLE.get(i).copied().flatten(),
    ));
    general.add(&reconnect);

    dialog.add(&general);

    let security = adw::PreferencesPage::builder()
        .title("Security")
        .icon_name("security-high-symbolic")
        .build();
    let kill = adw::PreferencesGroup::new();
    let kill_switch = switch(app, "Kill Switch", |p| p.kill_switch, |p, v| p.kill_switch = v);
    kill_switch.set_subtitle(
        "Block internet traffic outside the VPN while connected, and keep it blocked if the connection drops.",
    );
    let lan = switch(
        app,
        "Allow Local Network Access",
        |p| p.kill_switch_allow_lan,
        |p, v| p.kill_switch_allow_lan = v,
    );
    kill_switch
        .bind_property("active", &lan, "sensitive")
        .sync_create()
        .build();
    kill.add(&kill_switch);
    kill.add(&lan);
    security.add(&kill);
    let leaks = adw::PreferencesGroup::builder().title("Leak Protection").build();
    leaks.add(&switch(
        app,
        "Block IPv6 When the VPN Has No IPv6",
        |p| p.block_ipv6,
        |p, v| p.block_ipv6 = v,
    ));
    security.add(&leaks);
    wifi_groups(app, &security);
    dialog.add(&security);

    let appearance = adw::PreferencesPage::builder()
        .title("Appearance")
        .icon_name("applications-graphics-symbolic")
        .build();
    let tray = adw::PreferencesGroup::builder().title("Tray Icon").build();
    tray.add(&choice(
        app,
        "Icon Style",
        &["Network", "Lock"],
        |p| usize::from(p.icon_set == "lock"),
        |p, i| p.icon_set = if i == 1 { "lock".into() } else { "default".into() },
    ));
    tray.add(&choice(
        app,
        "Address",
        &["IPv4", "IPv6"],
        |p| usize::from(p.address_display == AddressDisplay::Ipv6),
        |p, i| {
            p.address_display = if i == 1 {
                AddressDisplay::Ipv6
            } else {
                AddressDisplay::Ipv4
            }
        },
    ));
    tray.add(&choice(
        app,
        "Traffic",
        &["Rates", "Totals"],
        |p| usize::from(p.traffic_display == TrafficDisplay::Totals),
        |p, i| {
            p.traffic_display = if i == 1 {
                TrafficDisplay::Totals
            } else {
                TrafficDisplay::Rates
            }
        },
    ));
    appearance.add(&tray);
    let window = adw::PreferencesGroup::builder().title("Details").build();
    window.add(&choice(
        app,
        "Traffic Graph",
        &["Line", "Bars", "Off"],
        |p| match p.graph_style {
            GraphStyle::Line => 0,
            GraphStyle::Bars => 1,
            GraphStyle::None => 2,
        },
        |p, i| {
            p.graph_style = match i {
                1 => GraphStyle::Bars,
                2 => GraphStyle::None,
                _ => GraphStyle::Line,
            }
        },
    ));
    appearance.add(&window);
    dialog.add(&appearance);

    let advanced = adw::PreferencesPage::builder()
        .title("Advanced")
        .icon_name("applications-engineering-symbolic")
        .build();
    let openvpn = adw::PreferencesGroup::builder().title("OpenVPN").build();
    let mut versions = vec!["Default".to_string()];
    versions.extend(app.versions.borrow().iter().cloned());
    let current = app.prefs.borrow().openvpn_version.clone();
    if let Some(v) = &current
        && !versions.contains(v)
    {
        versions.push(v.clone());
    }
    let labels: Vec<&str> = versions.iter().map(String::as_str).collect();
    let version = adw::ComboRow::builder()
        .title("Version")
        .model(&gtk::StringList::new(&labels))
        .selected(current.and_then(|v| versions.iter().position(|x| *x == v)).unwrap_or(0) as u32)
        .build();
    let a = app.clone();
    version.connect_selected_notify(move |r| {
        a.prefs.borrow_mut().openvpn_version = match r.selected() {
            0 => None,
            i => versions.get(i as usize).cloned(),
        };
        a.save_prefs();
    });
    openvpn.add(&version);
    let unsafe_row = adw::ActionRow::builder()
        .title("Unsafe OpenVPN Commands")
        .subtitle("Allow scripts and plugins in configurations with: sudo fluidity system-settings --allow-unsafe-commands true")
        .subtitle_selectable(true)
        .build();
    openvpn.add(&unsafe_row);
    advanced.add(&openvpn);

    let data = adw::PreferencesGroup::builder().title("Data").build();
    let folder = adw::ActionRow::builder()
        .title("Connections Folder")
        .subtitle(app.store.root().to_string_lossy().as_ref())
        .activatable(true)
        .build();
    folder.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
    let root = app.store.root().to_path_buf();
    let a = app.clone();
    folder.connect_activated(move |_| {
        let file = gtk::gio::File::for_path(&root);
        gtk::FileLauncher::new(Some(&file)).launch(Some(&a.parent()), gtk::gio::Cancellable::NONE, |_| {});
    });
    data.add(&folder);
    let forget = adw::ActionRow::builder().title("Saved Passwords").build();
    let forget_button = gtk::Button::builder()
        .label("Forget All")
        .valign(gtk::Align::Center)
        .css_classes(["destructive-action"])
        .build();
    forget.add_suffix(&forget_button);
    let a = app.clone();
    let d = dialog.clone();
    forget_button.connect_clicked(move |_| {
        let ids: Vec<String> = a.connections.borrow().iter().map(|c| c.id.clone()).collect();
        for id in ids {
            a.forget(&id);
        }
        d.add_toast(adw::Toast::new("Saved passwords removed"));
    });
    data.add(&forget);
    advanced.add(&data);
    dialog.add(&advanced);

    dialog.present(Some(&app.parent()));
}

pub fn new_folder(app: &Rc<App>) {
    let dialog = adw::AlertDialog::new(Some("New Folder"), None);
    let entry = adw::EntryRow::builder().title("Name").activates_default(true).build();
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).build();
    list.append(&entry);
    dialog.set_extra_child(Some(&list));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");
    dialog.set_response_enabled("create", false);
    let d = dialog.clone();
    entry.connect_changed(move |e| d.set_response_enabled("create", !e.text().trim().is_empty()));
    let a = app.clone();
    dialog.connect_response(Some("create"), move |_, _| {
        let mut tree = a.tree.borrow().clone();
        tree.push(Node::Folder(Folder {
            id: format!(
                "folder-{}",
                glib::uuid_string_random().replace('-', "")[..12].to_owned()
            ),
            name: entry.text().trim().to_string(),
            children: Vec::new(),
            shared_credentials: false,
            before_connect_script: None,
            connected_script: None,
            disconnected_script: None,
        }));
        a.set_tree(tree);
    });
    dialog.present(Some(&app.parent()));
}

pub fn move_dialog(app: &Rc<App>, id: &str) {
    let mut folders = Vec::new();
    state::walk_folders(&app.tree.borrow(), &mut folders);
    if folders.is_empty() {
        app.error("Create a folder first.");
        return;
    }
    let current = app.connection(id).and_then(|c| c.folder);
    let mut labels = vec!["No Folder".to_string()];
    labels.extend(folders.iter().map(|f| f.1.clone()));
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let combo = adw::ComboRow::builder()
        .title("Folder")
        .model(&gtk::StringList::new(&refs))
        .selected(
            current
                .and_then(|c| folders.iter().position(|f| f.0 == c))
                .map(|i| i as u32 + 1)
                .unwrap_or(0),
        )
        .build();
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).build();
    list.append(&combo);
    let dialog = adw::AlertDialog::new(Some("Move to Folder"), None);
    dialog.set_extra_child(Some(&list));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("move", "Move");
    dialog.set_response_appearance("move", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("move"));
    dialog.set_close_response("cancel");
    let a = app.clone();
    let id = id.to_string();
    dialog.connect_response(Some("move"), move |_, _| {
        let target = combo
            .selected()
            .checked_sub(1)
            .and_then(|i| folders.get(i as usize))
            .map(|f| f.0.clone());
        let mut tree = a.tree.borrow().clone();
        state::move_to(&mut tree, &id, target.as_deref());
        a.set_tree(tree);
    });
    dialog.present(Some(&app.parent()));
}

pub fn folder_dialog(app: &Rc<App>, id: &str) {
    let Some(folder) = app.store.folder(id) else { return };
    let dialog = adw::Dialog::builder()
        .title("Folder Settings")
        .content_width(480)
        .build();
    let page = adw::PreferencesPage::new();
    let main = adw::PreferencesGroup::new();
    let name = adw::EntryRow::builder().title("Name").text(&folder.name).build();
    let shared = adw::SwitchRow::builder()
        .title("Share Saved Passwords")
        .subtitle("All connections in this folder use the same saved credentials.")
        .active(folder.shared_credentials)
        .build();
    main.add(&name);
    main.add(&shared);
    page.add(&main);
    let scripts = adw::PreferencesGroup::builder()
        .title("Scripts")
        .description("These run for every connection in the folder that has no script of its own.")
        .build();
    let entries = [
        ("Before Connect", folder.before_connect_script.clone()),
        ("Connected", folder.connected_script.clone()),
        ("Disconnected", folder.disconnected_script.clone()),
    ]
    .map(|(title, value)| {
        let row = adw::EntryRow::builder()
            .title(title)
            .text(value.unwrap_or_default())
            .build();
        scripts.add(&row);
        row
    });
    page.add(&scripts);
    let danger = adw::PreferencesGroup::new();
    let remove = gtk::Button::builder()
        .label("Remove Folder")
        .halign(gtk::Align::Center)
        .css_classes(["pill", "destructive-action"])
        .build();
    danger.add(&remove);
    page.add(&danger);

    let header = adw::HeaderBar::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));

    let a = app.clone();
    let folder_id = id.to_string();
    let d = dialog.clone();
    remove.connect_clicked(move |_| {
        let mut tree = a.tree.borrow().clone();
        state::remove_folder(&mut tree, &folder_id);
        a.set_tree(tree);
        d.close();
    });
    let a = app.clone();
    let folder_id = id.to_string();
    dialog.connect_closed(move |_| {
        let mut tree = a.tree.borrow().clone();
        let Some(f) = state::folder_mut(&mut tree, &folder_id) else {
            return;
        };
        let text = |e: &adw::EntryRow| Some(e.text().trim().to_string()).filter(|s| !s.is_empty());
        if let Some(n) = text(&name) {
            f.name = n;
        }
        f.shared_credentials = shared.is_active();
        f.before_connect_script = text(&entries[0]);
        f.connected_script = text(&entries[1]);
        f.disconnected_script = text(&entries[2]);
        a.set_tree(tree);
    });
    dialog.present(Some(&app.parent()));
}
