#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod backend;
mod editor;
mod prefs;
mod prompt;
mod scripts;
mod state;
mod tray;
mod wifi;
mod window;

use std::cell::{Cell, OnceCell};
use std::rc::Rc;

use adw::prelude::*;
use fluidity_core::Store;
use fluidity_core::deploy::{ConnectionMode, Deployment};
use gtk::{gio, glib};

use crate::app::App;
use crate::prefs::APP_ID;

const WEBSITE: &str = "https://github.com/zlotny/fluidity-vpn";

thread_local! {
    static APP: OnceCell<Rc<App>> = const { OnceCell::new() };
    static HIDDEN: Cell<bool> = const { Cell::new(false) };
}

type AppAction = Box<dyn Fn(&Rc<App>)>;

fn app() -> Option<Rc<App>> {
    APP.with(|a| a.get().cloned())
}

fn main() -> glib::ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--hidden") {
        args.remove(i);
        HIDDEN.with(|h| h.set(true));
    }
    let gtk_app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    gtk_app.connect_startup(startup);
    gtk_app.connect_activate(|_| {
        if HIDDEN.with(|h| h.replace(false)) {
            return;
        }
        if let Some(app) = app() {
            app.show();
        }
    });
    gtk_app.connect_open(|_, files, _| {
        let Some(app) = app() else { return };
        let paths: Vec<_> = files.iter().filter_map(|f| f.path()).collect();
        let id = app.import_paths(&paths);
        app.show();
        if let Some(id) = id {
            app.window().select(&id);
        }
    });
    gtk_app.run_with_args(&args)
}

fn startup(gtk_app: &adw::Application) {
    gtk::Window::set_default_icon_name(APP_ID);
    std::mem::forget(gtk_app.hold());
    let store = match Store::open_default() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("fluidity: {e}");
            gtk_app.quit();
            return;
        }
    };
    let app = App::new(gtk_app.clone(), store);
    APP.with(|a| {
        let _ = a.set(app.clone());
    });
    install_actions(&app);
    load_css();
    deploy(&app);
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string("row.property .subtitle { font-feature-settings: \"tnum\"; }");
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}

fn install_actions(app: &Rc<App>) {
    let gtk_app = &app.gtk;
    let add = |name: &str, accels: &[&str], f: AppAction| {
        let action = gio::SimpleAction::new(name, None);
        let app = app.clone();
        action.connect_activate(move |_, _| f(&app));
        gtk_app.add_action(&action);
        if !accels.is_empty() {
            gtk_app.set_accels_for_action(&format!("app.{name}"), accels);
        }
    };
    add("show", &[], Box::new(|app| app.show()));
    add("import", &["<Control>o"], Box::new(|app| app.window().import()));
    add("new", &["<Control>n"], Box::new(|app| editor::open(app, None)));
    add("new-folder", &["<Control><Shift>n"], Box::new(prefs::new_folder));
    add("preferences", &["<Control>comma"], Box::new(prefs::open));
    add("connect-all", &[], Box::new(|app| app.connect_all()));
    add("disconnect-all", &[], Box::new(|app| app.disconnect_all()));
    add("about", &[], Box::new(about));
    add("quit", &["<Control>q"], Box::new(quit));
    gtk_app.set_accels_for_action("window.close", &["<Control>w"]);
    gtk_app.set_accels_for_action("win.toggle", &["<Control>Return"]);
    gtk_app.set_accels_for_action("win.edit", &["<Control>e"]);
}

fn about(app: &Rc<App>) {
    let dialog = adw::AboutDialog::builder()
        .application_name("Fluidity")
        .application_icon(APP_ID)
        .developer_name("The Fluidity Contributors")
        .version(env!("CARGO_PKG_VERSION"))
        .website(WEBSITE)
        .issue_url(format!("{WEBSITE}/issues"))
        .license_type(gtk::License::_0bsd)
        .comments("A free and open source OpenVPN client.")
        .build();
    dialog.present(Some(&app.parent()));
}

fn quit(app: &Rc<App>) {
    if app.active_ids().is_empty() {
        app.gtk.quit();
        return;
    }
    let dialog = adw::AlertDialog::new(
        Some("Disconnect Before Quitting?"),
        Some("Connections stay up after Fluidity quits unless you disconnect them."),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("keep", "Stay Connected");
    dialog.add_response("disconnect", "Disconnect");
    dialog.set_response_appearance("disconnect", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("disconnect"));
    dialog.set_close_response("cancel");
    let a = app.clone();
    dialog.connect_response(None, move |_, response| match response {
        "disconnect" => {
            a.disconnect_all();
            let a = a.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || a.gtk.quit());
        }
        "keep" => a.gtk.quit(),
        _ => {}
    });
    dialog.present(Some(&app.parent()));
}

fn deploy(app: &Rc<App>) {
    let Some(pending) = Deployment::pending(&app.store, &fluidity_core::deploy::system_dirs()) else {
        return;
    };
    if pending.manifest.connections != ConnectionMode::Prompt {
        apply_deployment(app, &pending, Some(pending.manifest.connections));
        return;
    }
    let dialog = adw::AlertDialog::new(
        Some("Install Connections From Your Organization?"),
        Some(&format!(
            "{} preconfigured connection(s) are available.",
            pending.connections.len()
        )),
    );
    dialog.add_response("skip", "Skip");
    dialog.add_response("append", "Add New Only");
    dialog.add_response("replace", "Replace Existing");
    dialog.set_response_appearance("replace", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("skip");
    let a = app.clone();
    dialog.connect_response(None, move |_, response| {
        let mode = match response {
            "replace" => Some(ConnectionMode::Replace),
            "append" => Some(ConnectionMode::Append),
            _ => None,
        };
        apply_deployment(&a, &pending, mode);
    });
    dialog.present(Some(&app.parent()));
}

fn apply_deployment(app: &Rc<App>, deployment: &Deployment, mode: Option<ConnectionMode>) {
    let result = match mode {
        Some(mode) => deployment.apply(&app.store, mode).map(|r| r.errors),
        None => deployment.mark(&app.store).map(|_| Vec::new()),
    };
    match result {
        Ok(errors) if !errors.is_empty() => app.error(&errors.join("\n")),
        Ok(_) => {}
        Err(e) => app.error(&format!("Cannot apply the deployment: {e}")),
    }
    *app.prefs.borrow_mut() = fluidity_core::Preferences::load(app.store.root());
    app.reload();
}
