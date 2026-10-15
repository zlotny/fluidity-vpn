use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use fluidity_core::{GraphStyle, Node};
use fluidity_ipc::{LogEntry, SessionState};
use gtk::{gio, glib};

use crate::app::App;
use crate::state;
use crate::tray::state_label;

type WinAction = Box<dyn Fn(&Rc<App>, &Rc<Window>, String)>;

struct Row {
    row: gtk::ListBoxRow,
    subtitle: gtk::Label,
    dot: gtk::Label,
}

struct Detail {
    title: adw::WindowTitle,
    icon: gtk::Image,
    name: gtk::Label,
    status: gtk::Label,
    error: gtk::Label,
    button: gtk::Button,
    rows: HashMap<&'static str, adw::ActionRow>,
    graph: gtk::DrawingArea,
    rates: gtk::Label,
    log: gtk::TextView,
    menu: gtk::MenuButton,
    stack: adw::ViewStack,
    usage_rows: Vec<adw::ActionRow>,
    usage_graph: gtk::DrawingArea,
    usage_reset: gtk::Button,
    check_group: adw::PreferencesGroup,
    check_rows: Vec<adw::ActionRow>,
    check_button: gtk::Button,
}

pub struct Window {
    pub win: adw::ApplicationWindow,
    app: std::rc::Weak<App>,
    list: gtk::ListBox,
    rows: RefCell<HashMap<String, Row>>,
    selected: RefCell<Option<String>>,
    toasts: adw::ToastOverlay,
    content_stack: gtk::Stack,
    split: adw::NavigationSplitView,
    detail: Detail,
    banner: adw::Banner,
    check: RefCell<Option<(String, crate::check::Report)>>,
}

const STATUS_ROWS: [(&str, &str); 10] = [
    ("state", "State"),
    ("duration", "Connected For"),
    ("ipv4", "IPv4 Address"),
    ("ipv6", "IPv6 Address"),
    ("server", "Server"),
    ("port", "Port"),
    ("interface", "Interface"),
    ("in", "Received"),
    ("out", "Sent"),
    ("killswitch", "Kill Switch"),
];

fn dot_class(state: SessionState) -> &'static str {
    match state {
        SessionState::Connected => "success",
        SessionState::Error => "error",
        SessionState::Disconnected => "dim-label",
        _ => "warning",
    }
}

fn status_icon(state: SessionState) -> &'static str {
    match state {
        SessionState::Connected => "network-vpn-symbolic",
        SessionState::Disconnected | SessionState::Error => "network-vpn-disconnected-symbolic",
        _ => "network-vpn-acquiring-symbolic",
    }
}

fn menu_model() -> gio::Menu {
    let menu = gio::Menu::new();
    let edit = gio::Menu::new();
    edit.append(Some("Edit…"), Some("win.edit"));
    edit.append(Some("Duplicate"), Some("win.duplicate"));
    edit.append(Some("Export…"), Some("win.export"));
    menu.append_section(None, &edit);
    let order = gio::Menu::new();
    order.append(Some("Move Up"), Some("win.move-up"));
    order.append(Some("Move Down"), Some("win.move-down"));
    order.append(Some("Move to Folder…"), Some("win.move-folder"));
    menu.append_section(None, &order);
    let danger = gio::Menu::new();
    danger.append(Some("Forget Saved Passwords"), Some("win.forget"));
    danger.append(Some("Remove…"), Some("win.remove"));
    menu.append_section(None, &danger);
    menu
}

fn primary_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let all = gio::Menu::new();
    all.append(Some("Connect All"), Some("app.connect-all"));
    all.append(Some("Disconnect All"), Some("app.disconnect-all"));
    menu.append_section(None, &all);
    let general = gio::Menu::new();
    general.append(Some("Preferences"), Some("app.preferences"));
    general.append(Some("About Fluidity"), Some("app.about"));
    menu.append_section(None, &general);
    let quit = gio::Menu::new();
    quit.append(Some("Quit"), Some("app.quit"));
    menu.append_section(None, &quit);
    menu
}

fn add_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Import Connection…"), Some("app.import"));
    menu.append(Some("New Connection…"), Some("app.new"));
    menu.append(Some("New Folder…"), Some("app.new-folder"));
    menu
}

impl Window {
    pub fn new(app: &Rc<App>) -> Rc<Self> {
        let win = adw::ApplicationWindow::builder()
            .application(&app.gtk)
            .title("Fluidity")
            .default_width(880)
            .default_height(600)
            .width_request(360)
            .height_request(300)
            .build();
        win.set_hide_on_close(true);

        let list = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        let side_header = adw::HeaderBar::new();
        side_header.pack_start(
            &gtk::MenuButton::builder()
                .icon_name("list-add-symbolic")
                .tooltip_text("Add Connection")
                .menu_model(&add_menu())
                .build(),
        );
        side_header.pack_end(
            &gtk::MenuButton::builder()
                .icon_name("open-menu-symbolic")
                .tooltip_text("Main Menu")
                .primary(true)
                .menu_model(&primary_menu())
                .build(),
        );
        let side_view = adw::ToolbarView::new();
        side_view.add_top_bar(&side_header);
        side_view.set_content(Some(&scroller));
        let sidebar = adw::NavigationPage::new(&side_view, "Fluidity");

        let (detail, detail_widget) = Self::detail();
        let empty = adw::StatusPage::builder()
            .icon_name("network-vpn-symbolic")
            .title("No Connections")
            .description("Import a configuration file or create a connection.")
            .build();
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .halign(gtk::Align::Center)
            .build();
        let import = gtk::Button::builder()
            .label("Import Connection…")
            .action_name("app.import")
            .css_classes(["pill", "suggested-action"])
            .build();
        let create = gtk::Button::builder()
            .label("New Connection…")
            .action_name("app.new")
            .css_classes(["pill"])
            .build();
        buttons.append(&import);
        buttons.append(&create);
        let hint = gtk::Label::builder()
            .label("Fluidity opens .ovpn, .conf, .fvpn, .fvpnz, .tblk and .zip files.")
            .css_classes(["dim-label", "caption"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        buttons.append(&hint);
        empty.set_child(Some(&buttons));
        let empty_view = adw::ToolbarView::new();
        empty_view.add_top_bar(&adw::HeaderBar::builder().show_title(false).build());
        empty_view.set_content(Some(&empty));

        let unselected = adw::StatusPage::builder()
            .icon_name("network-vpn-symbolic")
            .title("Select a Connection")
            .build();
        let unselected_view = adw::ToolbarView::new();
        unselected_view.add_top_bar(&adw::HeaderBar::builder().show_title(false).build());
        unselected_view.set_content(Some(&unselected));

        let content_stack = gtk::Stack::new();
        content_stack.add_named(&detail_widget, Some("detail"));
        content_stack.add_named(&empty_view, Some("empty"));
        content_stack.add_named(&unselected_view, Some("unselected"));
        let content = adw::NavigationPage::new(&content_stack, "Connection");

        let split = adw::NavigationSplitView::builder()
            .sidebar(&sidebar)
            .content(&content)
            .min_sidebar_width(220.0)
            .build();
        let banner = adw::Banner::builder()
            .title("The kill switch is blocking internet traffic")
            .button_label("Allow Traffic")
            .build();
        let weak = Rc::downgrade(app);
        banner.connect_button_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.unblock();
            }
        });
        let layout = gtk::Box::new(gtk::Orientation::Vertical, 0);
        split.set_vexpand(true);
        layout.append(&banner);
        layout.append(&split);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&layout));
        win.set_content(Some(&toasts));

        let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 600sp").expect("condition"));
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        win.add_breakpoint(breakpoint);

        let window = Rc::new(Self {
            win,
            app: Rc::downgrade(app),
            list,
            rows: RefCell::default(),
            selected: RefCell::default(),
            toasts,
            content_stack,
            split,
            detail,
            banner,
            check: RefCell::default(),
        });
        window.wire(app);
        window.rebuild();
        window
    }

    fn detail() -> (Detail, gtk::Widget) {
        let title = adw::WindowTitle::new("", "");
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));
        let menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text("Connection Menu")
            .menu_model(&menu_model())
            .build();
        header.pack_end(&menu);

        let icon = gtk::Image::builder().pixel_size(48).build();
        let name = gtk::Label::builder()
            .css_classes(["title-2"])
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let status = gtk::Label::builder()
            .css_classes(["dim-label"])
            .xalign(0.0)
            .wrap(true)
            .build();
        let error = gtk::Label::builder()
            .css_classes(["error"])
            .xalign(0.0)
            .wrap(true)
            .visible(false)
            .build();
        let labels = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        labels.append(&name);
        labels.append(&status);
        labels.append(&error);
        let button = gtk::Button::builder()
            .css_classes(["pill"])
            .valign(gtk::Align::Center)
            .action_name("win.toggle")
            .build();
        let top = gtk::Box::builder().spacing(16).build();
        top.append(&icon);
        top.append(&labels);
        top.append(&button);

        let stack = adw::ViewStack::new();
        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .halign(gtk::Align::Center)
            .build();

        let group = adw::PreferencesGroup::new();
        let mut rows = HashMap::new();
        for (key, label) in STATUS_ROWS {
            let row = adw::ActionRow::builder()
                .title(label)
                .subtitle_selectable(true)
                .use_markup(false)
                .css_classes(["property"])
                .build();
            group.add(&row);
            rows.insert(key, row);
        }
        let check_button = gtk::Button::builder()
            .label("Check")
            .valign(gtk::Align::Center)
            .action_name("win.check")
            .build();
        let check_group = adw::PreferencesGroup::builder()
            .title("Connection Check")
            .description("See the address websites see, and the latency to the server.")
            .header_suffix(&check_button)
            .build();
        let check_rows: Vec<adw::ActionRow> = ["Public IP", "Location", "Network", "Server Latency"]
            .into_iter()
            .map(|title| {
                let row = adw::ActionRow::builder()
                    .title(title)
                    .subtitle_selectable(true)
                    .use_markup(false)
                    .css_classes(["property"])
                    .visible(false)
                    .build();
                check_group.add(&row);
                row
            })
            .collect();
        let status_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .build();
        status_box.append(&group);
        status_box.append(&check_group);
        let status_page = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&status_box)
            .build();
        stack.add_titled_with_icon(&status_page, Some("status"), "Status", "network-wired-symbolic");

        let graph = gtk::DrawingArea::builder()
            .vexpand(true)
            .hexpand(true)
            .content_height(200)
            .build();
        let frame = gtk::Frame::builder().child(&graph).build();
        let rates = gtk::Label::builder().css_classes(["dim-label", "numeric"]).build();
        let traffic = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .build();
        traffic.append(&frame);
        traffic.append(&rates);
        stack.add_titled_with_icon(
            &traffic,
            Some("traffic"),
            "Traffic",
            "power-profile-performance-symbolic",
        );

        let usage_group = adw::PreferencesGroup::new();
        let usage_rows: Vec<adw::ActionRow> = ["Today", "Last 7 days", "Last 30 days"]
            .into_iter()
            .map(|title| {
                let row = adw::ActionRow::builder()
                    .title(title)
                    .use_markup(false)
                    .css_classes(["property"])
                    .build();
                usage_group.add(&row);
                row
            })
            .collect();
        let usage_graph = gtk::DrawingArea::builder().hexpand(true).content_height(180).build();
        let usage_frame = gtk::Frame::builder().child(&usage_graph).build();
        let usage_note = gtk::Label::builder()
            .label("Usage is kept on this computer for 30 days.")
            .css_classes(["dim-label"])
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .build();
        let usage_reset = gtk::Button::builder()
            .label("Reset…")
            .action_name("win.reset-usage")
            .build();
        let usage_actions = gtk::Box::builder().spacing(8).build();
        usage_actions.append(&usage_note);
        usage_actions.append(&usage_reset);
        let usage_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .build();
        usage_box.append(&usage_group);
        usage_box.append(&usage_frame);
        usage_box.append(&usage_actions);
        let usage_page = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&usage_box)
            .build();
        stack.add_titled_with_icon(&usage_page, Some("usage"), "Usage", "x-office-spreadsheet-symbolic");

        let log = gtk::TextView::builder()
            .editable(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(8)
            .right_margin(8)
            .build();
        let log_scroll = gtk::ScrolledWindow::builder().vexpand(true).child(&log).build();
        let log_frame = gtk::Frame::builder().child(&log_scroll).build();
        let log_actions = gtk::Box::builder().spacing(8).halign(gtk::Align::End).build();
        log_actions.append(&gtk::Button::builder().label("Copy").action_name("win.copy-log").build());
        log_actions.append(
            &gtk::Button::builder()
                .label("Clear")
                .action_name("win.clear-log")
                .build(),
        );
        let log_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .build();
        log_box.append(&log_frame);
        log_box.append(&log_actions);
        stack.add_titled_with_icon(&log_box, Some("log"), "Log", "text-x-generic-symbolic");

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(18)
            .margin_end(18)
            .build();
        body.append(&top);
        body.append(&switcher);
        body.append(&stack);
        let clamp = adw::Clamp::builder().maximum_size(720).child(&body).build();
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&clamp));
        (
            Detail {
                title,
                icon,
                name,
                status,
                error,
                button,
                rows,
                graph,
                rates,
                log,
                menu,
                stack,
                usage_rows,
                usage_graph,
                usage_reset,
                check_group,
                check_rows,
                check_button,
            },
            view.upcast(),
        )
    }

    fn wire(self: &Rc<Self>, app: &Rc<App>) {
        let weak = Rc::downgrade(self);
        self.list.connect_row_selected(move |_, row| {
            let Some(w) = weak.upgrade() else { return };
            let id = row.map(|r| r.widget_name().to_string()).filter(|n| !n.is_empty());
            if id.is_some() {
                *w.selected.borrow_mut() = id.clone();
                w.show_selected();
                w.split.set_show_content(true);
            }
        });
        let weak = Rc::downgrade(self);
        self.list.connect_row_activated(move |_, row| {
            let Some(w) = weak.upgrade() else { return };
            if let Some(folder) = row.widget_name().strip_prefix("folder:") {
                crate::prefs::folder_dialog(&w.app(), folder);
            }
        });

        let action = |name: &str, f: WinAction| {
            let a = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            a.connect_activate(move |_, _| {
                let Some(w) = weak.upgrade() else { return };
                let Some(id) = w.selected.borrow().clone() else { return };
                f(&w.app(), &w, id);
            });
            self.win.add_action(&a);
        };
        action("toggle", Box::new(|app, _, id| app.toggle(&id)));
        action("edit", Box::new(|app, _, id| crate::editor::open(app, Some(id))));
        action(
            "duplicate",
            Box::new(|app, w, id| {
                if let Some(new) = app.duplicate(&id) {
                    w.select(&new);
                }
            }),
        );
        action("export", Box::new(|app, w, id| w.export(app, id)));
        action(
            "forget",
            Box::new(|app, w, id| {
                app.forget(&id);
                w.toast("Saved passwords removed");
            }),
        );
        action("remove", Box::new(|app, w, id| w.confirm_remove(app, id)));
        action(
            "move-up",
            Box::new(|app, _, id| {
                let mut tree = app.tree.borrow().clone();
                state::shift(&mut tree, &id, -1);
                app.set_tree(tree);
            }),
        );
        action(
            "move-down",
            Box::new(|app, _, id| {
                let mut tree = app.tree.borrow().clone();
                state::shift(&mut tree, &id, 1);
                app.set_tree(tree);
            }),
        );
        action(
            "move-folder",
            Box::new(|app, _, id| crate::prefs::move_dialog(app, &id)),
        );
        action(
            "copy-log",
            Box::new(|app, w, id| {
                let text = log_text(app.logs.borrow().get(&id).map(Vec::as_slice).unwrap_or_default());
                w.win.clipboard().set_text(&text);
                w.toast("Log copied");
            }),
        );
        action(
            "clear-log",
            Box::new(|app, w, id| {
                app.logs.borrow_mut().insert(id, Vec::new());
                w.detail.log.buffer().set_text("");
            }),
        );

        action(
            "check",
            Box::new(|app, w, id| {
                let server = app
                    .sessions
                    .borrow()
                    .get(&id)
                    .map(|s| s.remote_address.clone())
                    .unwrap_or_default();
                w.detail.check_button.set_sensitive(false);
                w.detail.check_button.set_label("Checking…");
                let weak = Rc::downgrade(w);
                glib::spawn_future_local(async move {
                    let report = gio::spawn_blocking(move || crate::check::run(&server))
                        .await
                        .unwrap_or_default();
                    if let Some(w) = weak.upgrade() {
                        *w.check.borrow_mut() = Some((id, report));
                        w.detail.check_button.set_sensitive(true);
                        w.detail.check_button.set_label("Check Again");
                        w.refresh_detail();
                    }
                });
            }),
        );

        let page = gio::SimpleAction::new("page", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        page.connect_activate(move |_, value| {
            let (Some(w), Some(name)) = (weak.upgrade(), value.and_then(|v| v.str())) else {
                return;
            };
            w.detail.stack.set_visible_child_name(name);
        });
        self.win.add_action(&page);
        action(
            "reset-usage",
            Box::new(|app, w, id| {
                let name = app.connection(&id).map(|c| c.name).unwrap_or_default();
                let dialog = adw::AlertDialog::new(
                    Some(&format!("Reset usage of “{name}”?")),
                    Some("The data usage history of this connection is deleted."),
                );
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("reset", "Reset");
                dialog.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
                dialog.set_close_response("cancel");
                let app = app.clone();
                dialog.connect_response(Some("reset"), move |_, _| {
                    app.send(crate::backend::Command::ClearUsage(id.clone()))
                });
                dialog.present(Some(&w.win));
            }),
        );

        let weak = Rc::downgrade(self);
        self.detail.usage_graph.set_draw_func(move |_, cr, width, height| {
            if let Some(w) = weak.upgrade() {
                w.draw_usage(cr, width as f64, height as f64);
            }
        });
        let weak = Rc::downgrade(self);
        self.detail.stack.connect_visible_child_name_notify(move |_| {
            if let Some(w) = weak.upgrade() {
                w.request_usage();
            }
        });

        let weak = Rc::downgrade(self);
        self.detail.graph.set_draw_func(move |_, cr, width, height| {
            if let Some(w) = weak.upgrade() {
                w.draw_graph(cr, width as f64, height as f64);
            }
        });

        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(1, move || {
            let Some(w) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if w.win.is_visible() {
                w.refresh_detail();
                if (state::now() as u64).is_multiple_of(30) {
                    w.request_usage();
                }
            }
            glib::ControlFlow::Continue
        });

        let drop = gtk::DropTarget::new(gio::File::static_type(), gtk::gdk::DragAction::COPY);
        drop.set_types(&[gtk::gdk::FileList::static_type()]);
        let weak = Rc::downgrade(app);
        drop.connect_drop(move |_, value, _, _| {
            let Some(app) = weak.upgrade() else { return false };
            let Ok(files) = value.get::<gtk::gdk::FileList>() else {
                return false;
            };
            let paths: Vec<_> = files.files().iter().filter_map(|f| f.path()).collect();
            if let Some(id) = app.import_paths(&paths) {
                app.window().select(&id);
            }
            true
        });
        self.win.add_controller(drop);
    }

    fn app(&self) -> Rc<App> {
        self.app.upgrade().expect("app")
    }

    pub fn toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        toast.set_timeout(5);
        self.toasts.add_toast(toast);
    }

    pub fn select(&self, id: &str) {
        *self.selected.borrow_mut() = Some(id.to_string());
        if let Some(row) = self.rows.borrow().get(id) {
            self.list.select_row(Some(&row.row));
        }
        self.show_selected();
        self.split.set_show_content(true);
    }

    pub fn rebuild(&self) {
        let app = self.app();
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = HashMap::new();
        fn add(w: &Window, app: &App, nodes: &[Node], depth: i32, rows: &mut HashMap<String, Row>) {
            for node in nodes {
                match node {
                    Node::Folder(f) => {
                        let label = gtk::Label::builder()
                            .label(&f.name)
                            .xalign(0.0)
                            .hexpand(true)
                            .css_classes(["heading", "dim-label"])
                            .build();
                        let icon = gtk::Image::from_icon_name("folder-symbolic");
                        let settings = gtk::Image::from_icon_name("emblem-system-symbolic");
                        settings.add_css_class("dim-label");
                        let line = gtk::Box::builder().spacing(8).margin_start(depth * 12).build();
                        line.append(&icon);
                        line.append(&label);
                        line.append(&settings);
                        let row = gtk::ListBoxRow::builder()
                            .child(&line)
                            .selectable(false)
                            .name(format!("folder:{}", f.id))
                            .tooltip_text("Folder Settings")
                            .build();
                        w.list.append(&row);
                        add(w, app, &f.children, depth + 1, rows);
                    }
                    Node::Connection(id) => {
                        let Some(c) = app.connection(id) else { continue };
                        let dot = gtk::Label::builder().label("●").valign(gtk::Align::Center).build();
                        let title = gtk::Label::builder()
                            .label(&c.name)
                            .xalign(0.0)
                            .ellipsize(gtk::pango::EllipsizeMode::End)
                            .build();
                        let subtitle = gtk::Label::builder()
                            .xalign(0.0)
                            .css_classes(["caption", "dim-label"])
                            .ellipsize(gtk::pango::EllipsizeMode::End)
                            .build();
                        let labels = gtk::Box::builder()
                            .orientation(gtk::Orientation::Vertical)
                            .hexpand(true)
                            .build();
                        labels.append(&title);
                        labels.append(&subtitle);
                        let line = gtk::Box::builder()
                            .spacing(10)
                            .margin_start(depth * 12)
                            .margin_top(4)
                            .margin_bottom(4)
                            .build();
                        line.append(&dot);
                        line.append(&labels);
                        let row = gtk::ListBoxRow::builder().child(&line).name(id.as_str()).build();
                        w.list.append(&row);
                        rows.insert(id.clone(), Row { row, subtitle, dot });
                    }
                }
            }
        }
        add(self, &app, &app.tree.borrow(), 0, &mut rows);
        *self.rows.borrow_mut() = rows;
        let selected = self.selected.borrow().clone();
        let valid = selected.filter(|id| self.rows.borrow().contains_key(id));
        let pick = valid.or_else(|| app.connections.borrow().first().map(|c| c.id.clone()));
        *self.selected.borrow_mut() = pick.clone();
        if let Some(id) = pick
            && let Some(r) = self.rows.borrow().get(&id)
        {
            self.list.select_row(Some(&r.row));
        }
        self.refresh();
        self.show_selected();
    }

    pub fn refresh(&self) {
        let app = self.app();
        self.banner.set_revealed(!app.blocked.borrow().is_empty());
        for (id, row) in self.rows.borrow().iter() {
            let sessions = app.sessions.borrow();
            let session = sessions.get(id);
            let state = session.map(|s| s.state).unwrap_or_default();
            let text = match session {
                Some(s) if state == SessionState::Connected && !s.local_ipv4.is_empty() => {
                    format!("Connected, {}", s.local_ipv4)
                }
                _ => state_label(state).to_string(),
            };
            row.subtitle.set_label(&text);
            for class in ["success", "error", "dim-label", "warning"] {
                row.dot.remove_css_class(class);
            }
            row.dot.add_css_class(dot_class(state));
        }
        self.refresh_detail();
    }

    fn show_selected(&self) {
        let app = self.app();
        let empty = app.connections.borrow().is_empty();
        let page = match self.selected.borrow().as_ref() {
            _ if empty => "empty",
            Some(_) => "detail",
            None => "unselected",
        };
        self.content_stack.set_visible_child_name(page);
        if page != "detail" {
            return;
        }
        let id = self.selected.borrow().clone().unwrap_or_default();
        let name = app.connection(&id).map(|c| c.name).unwrap_or_default();
        self.detail.title.set_title(&name);
        self.detail.name.set_label(&name);
        if !app.logs.borrow().contains_key(&id) && app.daemon.get() {
            app.send(crate::backend::Command::Log(id.clone()));
        }
        self.log_changed(&id);
        self.usage_changed(&id);
        self.request_usage();
        self.refresh_detail();
    }

    fn request_usage(&self) {
        let app = self.app();
        if self.detail.stack.visible_child_name().as_deref() != Some("usage") || !app.daemon.get() {
            return;
        }
        if let Some(id) = self.selected.borrow().clone() {
            app.send(crate::backend::Command::Usage(id));
        }
    }

    fn usage_days(&self, id: &str) -> Vec<(u64, u64)> {
        let app = self.app();
        let usage = app.usage.borrow();
        let samples = usage.get(id).map(Vec::as_slice).unwrap_or_default();
        let day_of = |t: u64| {
            let offset = glib::DateTime::from_unix_local(t as i64)
                .map(|d| d.utc_offset().as_seconds())
                .unwrap_or_default();
            (t as i64 + offset).div_euclid(86400)
        };
        state::usage_days(samples, 30, day_of(state::now() as u64), day_of)
    }

    pub fn usage_changed(&self, id: &str) {
        if self.selected.borrow().as_deref() != Some(id) {
            return;
        }
        let days = self.usage_days(id);
        for (row, count) in self.detail.usage_rows.iter().zip([1, 7, 30]) {
            let (i, o) = state::usage_total(&days[days.len() - count..]);
            row.set_subtitle(&format!("↓ {}   ↑ {}", state::bytes(i as f64), state::bytes(o as f64)));
        }
        self.detail
            .usage_reset
            .set_sensitive(days.iter().any(|(i, o)| i + o > 0));
        self.detail.usage_graph.queue_draw();
    }

    fn draw_usage(&self, cr: &gtk::cairo::Context, width: f64, height: f64) {
        let Some(id) = self.selected.borrow().clone() else {
            return;
        };
        let days = self.usage_days(&id);
        let fg = self.win.color();
        let ink = |a: f64| cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, a);
        let peak = days.iter().map(|(i, o)| (i + o) as f64).fold(0.0, f64::max);
        if peak <= 0.0 {
            ink(0.5);
            let layout = self.win.create_pango_layout(Some("No data usage yet"));
            let (tw, th) = layout.pixel_size();
            cr.move_to((width - tw as f64) / 2.0, (height - th as f64) / 2.0);
            pangocairo::functions::show_layout(cr, &layout);
            return;
        }
        let ticks = state::ticks(peak);
        let label_width = ticks
            .iter()
            .map(|t| self.win.create_pango_layout(Some(&state::bytes(*t))).pixel_size().0)
            .max()
            .unwrap_or_default() as f64;
        let (left, top, bottom, right) = (10.0, 12.0, 24.0, label_width + 16.0);
        let plot_w = width - left - right;
        let plot_h = height - top - bottom;
        let scale = peak.max(*ticks.last().unwrap_or(&peak));
        let y = |v: f64| top + plot_h - v / scale * plot_h;
        cr.set_line_width(1.0);
        for t in &ticks {
            ink(0.12);
            let ty = y(*t).round() + 0.5;
            cr.move_to(left, ty);
            cr.line_to(left + plot_w, ty);
            let _ = cr.stroke();
            ink(0.6);
            let layout = self.win.create_pango_layout(Some(&state::bytes(*t)));
            let th = layout.pixel_size().1 as f64;
            cr.move_to(left + plot_w + 8.0, ty - th / 2.0);
            pangocairo::functions::show_layout(cr, &layout);
        }
        let slot = plot_w / days.len() as f64;
        let bar = (slot * 0.7).max(1.0);
        for (k, (i, o)) in days.iter().enumerate() {
            let x = left + k as f64 * slot + (slot - bar) / 2.0;
            cr.set_source_rgba(0.21, 0.52, 0.89, 0.9);
            cr.rectangle(x, y(*i as f64), bar, y(0.0) - y(*i as f64));
            let _ = cr.fill();
            cr.set_source_rgba(0.18, 0.76, 0.49, 0.9);
            cr.rectangle(x, y((i + o) as f64), bar, y(*i as f64) - y((i + o) as f64));
            let _ = cr.fill();
        }
        ink(0.6);
        let now = glib::DateTime::now_local().ok();
        for back in (6..days.len()).step_by(7) {
            let Some(date) = now.as_ref().and_then(|d| d.add_days(-(back as i32)).ok()) else {
                continue;
            };
            let text = date.format("%b %-d").map(|s| s.to_string()).unwrap_or_default();
            let layout = self.win.create_pango_layout(Some(text.trim()));
            let (tw, _) = layout.pixel_size();
            let center = left + (days.len() - 1 - back) as f64 * slot + slot / 2.0;
            cr.move_to(center - tw as f64 / 2.0, top + plot_h + 6.0);
            pangocairo::functions::show_layout(cr, &layout);
        }
    }

    fn refresh_detail(&self) {
        let Some(id) = self.selected.borrow().clone() else {
            return;
        };
        let app = self.app();
        let sessions = app.sessions.borrow();
        let session = sessions.get(&id);
        let state = session.map(|s| s.state).unwrap_or_default();
        let d = &self.detail;
        d.icon.set_icon_name(Some(status_icon(state)));
        let status = match session {
            Some(s) if !s.detail.is_empty() => format!("{}: {}", state_label(state), s.detail),
            _ => state_label(state).to_string(),
        };
        d.status.set_label(&status);
        let error = session
            .and_then(|s| s.last_error.clone())
            .filter(|_| !state.is_active());
        d.error.set_visible(error.is_some());
        d.error.set_label(error.as_deref().unwrap_or_default());
        let active = state.is_active();
        d.button.set_label(if active { "Disconnect" } else { "Connect" });
        d.button.set_sensitive(app.daemon.get() || active);
        if active {
            d.button.remove_css_class("suggested-action");
            d.button.add_css_class("destructive-action");
        } else {
            d.button.remove_css_class("destructive-action");
            d.button.add_css_class("suggested-action");
        }
        if !app.daemon.get() {
            d.status.set_label("The Fluidity service is not running");
        }
        let dash = |v: &str| if v.is_empty() { "–".to_string() } else { v.to_string() };
        let connected = session.filter(|s| s.state == SessionState::Connected);
        let values = [
            ("state", state_label(state).to_string()),
            (
                "duration",
                connected
                    .and_then(|s| s.connected_since)
                    .map(|t| state::duration((state::now() as u64).saturating_sub(t)))
                    .unwrap_or_else(|| "–".into()),
            ),
            ("ipv4", dash(session.map(|s| s.local_ipv4.as_str()).unwrap_or_default())),
            ("ipv6", dash(session.map(|s| s.local_ipv6.as_str()).unwrap_or_default())),
            (
                "server",
                dash(session.map(|s| s.remote_address.as_str()).unwrap_or_default()),
            ),
            (
                "port",
                dash(session.map(|s| s.remote_port.as_str()).unwrap_or_default()),
            ),
            (
                "interface",
                dash(session.map(|s| s.interface.as_str()).unwrap_or_default()),
            ),
            (
                "in",
                session
                    .map(|s| state::bytes(s.bytes_in as f64))
                    .unwrap_or_else(|| "–".into()),
            ),
            (
                "out",
                session
                    .map(|s| state::bytes(s.bytes_out as f64))
                    .unwrap_or_else(|| "–".into()),
            ),
            (
                "killswitch",
                match session {
                    Some(s) if s.kill_switch => "On".to_string(),
                    Some(_) => "Off".to_string(),
                    None => "–".to_string(),
                },
            ),
        ];
        for (key, value) in values {
            if let Some(row) = d.rows.get(key) {
                row.set_subtitle(&value);
            }
        }
        d.check_group.set_visible(connected.is_some());
        let check = self.check.borrow();
        let report = check.as_ref().filter(|(cid, _)| *cid == id).map(|(_, r)| r);
        if report.is_none() && d.check_button.is_sensitive() {
            d.check_button.set_label("Check");
        }
        let latency = report.map(|r| match r.latency {
            Some(ms) if ms < 0.1 => "< 0.1 ms".to_string(),
            Some(ms) if ms < 10.0 => format!("{ms:.1} ms"),
            Some(ms) => format!("{ms:.0} ms"),
            None => "No reply".into(),
        });
        let values = [
            report.map(|r| r.ip.clone().unwrap_or_else(|| "Unavailable".into())),
            report.and_then(|r| r.location.clone()),
            report.and_then(|r| r.network.clone()),
            latency,
        ];
        for (row, value) in d.check_rows.iter().zip(values) {
            row.set_visible(value.is_some());
            row.set_subtitle(value.as_deref().unwrap_or_default());
        }
        let traffic = app.traffic.borrow();
        let last = traffic.get(&id).and_then(|t| t.last().copied());
        let (i, o) = last.map(|t| (t.in_rate, t.out_rate)).unwrap_or_default();
        d.rates
            .set_label(&format!("In {}/s    Out {}/s", state::bytes(i), state::bytes(o)));
        d.graph.queue_draw();
        d.menu.set_sensitive(true);
    }

    fn draw_graph(&self, cr: &gtk::cairo::Context, width: f64, height: f64) {
        let app = self.app();
        let style = app.prefs.borrow().graph_style;
        let Some(id) = self.selected.borrow().clone() else {
            return;
        };
        let traffic = app.traffic.borrow();
        let samples = traffic.get(&id).map(Vec::as_slice).unwrap_or_default();
        let fg = self.win.color();
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.12);
        cr.set_line_width(1.0);
        for k in 1..4 {
            let y = (height * k as f64 / 4.0).round() + 0.5;
            cr.move_to(0.0, y);
            cr.line_to(width, y);
        }
        let _ = cr.stroke();
        if style == GraphStyle::None || samples.is_empty() {
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.5);
            let layout = self.win.create_pango_layout(Some(if style == GraphStyle::None {
                "Traffic graph is turned off"
            } else {
                "No traffic yet"
            }));
            let (tw, th) = layout.pixel_size();
            cr.move_to((width - tw as f64) / 2.0, (height - th as f64) / 2.0);
            pangocairo::functions::show_layout(cr, &layout);
            return;
        }
        let peak = samples.iter().map(|s| s.in_rate.max(s.out_rate)).fold(1024.0, f64::max) * 1.15;
        let step = width / (state::HISTORY - 1) as f64;
        let offset = state::HISTORY - samples.len();
        let series = [
            (
                (0.21, 0.52, 0.89),
                samples.iter().map(|s| s.in_rate).collect::<Vec<_>>(),
            ),
            (
                (0.18, 0.76, 0.49),
                samples.iter().map(|s| s.out_rate).collect::<Vec<_>>(),
            ),
        ];
        for ((r, g, b), values) in series {
            match style {
                GraphStyle::Bars => {
                    cr.set_source_rgba(r, g, b, 0.55);
                    for (k, v) in values.iter().enumerate() {
                        let x = (offset + k) as f64 * step;
                        let h = v / peak * height;
                        cr.rectangle(x, height - h, step.max(1.0), h);
                    }
                    let _ = cr.fill();
                }
                _ => {
                    cr.set_source_rgba(r, g, b, 1.0);
                    cr.set_line_width(2.0);
                    for (k, v) in values.iter().enumerate() {
                        let x = (offset + k) as f64 * step;
                        let y = height - v / peak * height;
                        if k == 0 {
                            cr.move_to(x, y);
                        } else {
                            cr.line_to(x, y);
                        }
                    }
                    let _ = cr.stroke();
                }
            }
        }
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.6);
        let layout = self.win.create_pango_layout(Some(&format!("{}/s", state::bytes(peak))));
        cr.move_to(6.0, 4.0);
        pangocairo::functions::show_layout(cr, &layout);
    }

    pub fn log_changed(&self, id: &str) {
        if self.selected.borrow().as_deref() != Some(id) {
            return;
        }
        let app = self.app();
        let logs = app.logs.borrow();
        let text = log_text(logs.get(id).map(Vec::as_slice).unwrap_or_default());
        self.detail.log.buffer().set_text(&text);
        self.scroll_log();
    }

    pub fn log_appended(&self, id: &str, entry: &LogEntry) {
        if self.selected.borrow().as_deref() != Some(id) {
            return;
        }
        let buffer = self.detail.log.buffer();
        let mut end = buffer.end_iter();
        buffer.insert(&mut end, &log_line(entry));
        self.scroll_log();
    }

    fn scroll_log(&self) {
        let buffer = self.detail.log.buffer();
        let mark = buffer.create_mark(None, &buffer.end_iter(), false);
        self.detail.log.scroll_mark_onscreen(&mark);
        buffer.delete_mark(&mark);
    }

    fn export(&self, app: &Rc<App>, id: String) {
        let name = app
            .connection(&id)
            .map(|c| c.name)
            .unwrap_or_else(|| "Connection".into());
        let dialog = gtk::FileDialog::builder()
            .title("Export Connection")
            .initial_name(format!("{name}.{}", fluidity_config::ARCHIVE_EXTENSION))
            .build();
        let app = app.clone();
        let parent = self.win.clone();
        glib::spawn_future_local(async move {
            let Ok(file) = dialog.save_future(Some(&parent)).await else {
                return;
            };
            let Some(path) = file.path() else { return };
            match app.store.export(&id, &path) {
                Ok(()) => app.window().toast(&format!("Exported {name}")),
                Err(e) => app.error(&e.to_string()),
            }
        });
    }

    pub fn import(&self) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("VPN Configurations"));
        for pattern in ["*.ovpn", "*.conf", "*.fvpnz", "*.zip", "*.tblk", "*.fvpn"] {
            filter.add_pattern(pattern);
        }
        let all = gtk::FileFilter::new();
        all.set_name(Some("All Files"));
        all.add_pattern("*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        filters.append(&all);
        let dialog = gtk::FileDialog::builder()
            .title("Import Connection")
            .filters(&filters)
            .default_filter(&filter)
            .build();
        let app = self.app();
        let parent = self.win.clone();
        parent.present();
        glib::spawn_future_local(async move {
            let Ok(files) = dialog.open_multiple_future(Some(&parent)).await else {
                return;
            };
            let paths: Vec<_> = (0..files.n_items())
                .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                .filter_map(|f| f.path())
                .collect();
            if let Some(id) = app.import_paths(&paths) {
                app.window().select(&id);
            }
        });
    }

    fn confirm_remove(&self, app: &Rc<App>, id: String) {
        let name = app.connection(&id).map(|c| c.name).unwrap_or_default();
        let dialog = adw::AlertDialog::new(
            Some(&format!("Remove “{name}”?")),
            Some("The connection, its files and its saved passwords are deleted."),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("remove", "Remove");
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        dialog.set_close_response("cancel");
        let app = app.clone();
        dialog.connect_response(Some("remove"), move |_, _| app.remove(&id));
        dialog.present(Some(&self.win));
    }
}

fn log_line(entry: &LogEntry) -> String {
    let time = glib::DateTime::from_unix_local(entry.time as i64)
        .ok()
        .and_then(|t| t.format("%Y-%m-%d %H:%M:%S").ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    format!("{time}  {}\n", entry.message)
}

fn log_text(entries: &[LogEntry]) -> String {
    entries.iter().map(log_line).collect()
}
