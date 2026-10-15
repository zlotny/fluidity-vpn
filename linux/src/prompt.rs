use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use fluidity_config::settings::StaticChallengePassword;
use fluidity_ipc::{Prompt, Reply};

use crate::app::App;

pub type Completion = Box<dyn Fn(Reply, bool)>;

pub fn dismiss(app: &App, id: &str) {
    let dialog = app.prompts.borrow_mut().remove(id);
    if let Some(dialog) = dialog {
        dialog.set_close_response("dismissed");
        dialog.close();
    }
}

pub fn dismiss_all(app: &App) {
    let ids: Vec<String> = app.prompts.borrow().keys().cloned().collect();
    for id in ids {
        dismiss(app, &id);
    }
}

fn heading(prompt: &Prompt) -> String {
    match prompt {
        Prompt::Credentials { retry: true, .. } => "Authentication Failed".into(),
        Prompt::Credentials { realm, .. } if realm == "Auth" => "Enter Your Credentials".into(),
        Prompt::Credentials { realm, .. } => format!("Enter Your {realm} Credentials"),
        Prompt::Password { realm } if realm == "Private Key" => "Enter the Private Key Passphrase".into(),
        Prompt::Password { realm } => format!("Enter the {realm} Password"),
        Prompt::Challenge { .. } => "Additional Authentication Required".into(),
        Prompt::NeedOk { .. } => "Confirmation Required".into(),
        Prompt::NeedStr { .. } => "Input Required".into(),
        Prompt::WebAuth { .. } => "Sign In With Your Browser".into(),
        Prompt::Choice { .. } => "Choose a Certificate".into(),
        Prompt::Certificate { .. } | Prompt::Sign { .. } => "Certificate Identity".into(),
    }
}

fn body(title: &str, prompt: &Prompt, splitter: Option<&StaticChallengePassword>) -> String {
    let detail = match prompt {
        Prompt::Credentials { retry: true, .. } => Some("Check your username and password, then try again.".into()),
        Prompt::Credentials {
            static_challenge: Some(_),
            ..
        } => splitter.filter(|s| !s.text.is_empty()).map(|s| {
            format!(
                "Enter your password, then {}, separated by \"{}\".",
                s.text, s.delimiter
            )
        }),
        Prompt::Challenge { text, .. } => Some(text.clone()),
        Prompt::NeedOk { message, .. } | Prompt::NeedStr { message, .. } => Some(message.clone()),
        Prompt::Choice { options, .. } if options.is_empty() => {
            Some("No tokens were found. Connect your token and try again.".into())
        }
        Prompt::Choice { message, .. } => Some(message.clone()),
        _ => None,
    };
    match detail {
        Some(d) => format!("{title}\n\n{d}"),
        None => title.to_string(),
    }
}

struct Fields {
    username: Option<adw::EntryRow>,
    password: Option<adw::PasswordEntryRow>,
    response: Option<gtk::Editable>,
    choice: Option<(adw::ComboRow, Vec<String>)>,
    remember: Option<adw::SwitchRow>,
}

fn response_row(title: &str, echo: bool) -> (gtk::Widget, gtk::Editable) {
    if echo {
        let row = adw::EntryRow::builder().title(title).build();
        (row.clone().upcast(), row.upcast())
    } else {
        let row = adw::PasswordEntryRow::builder().title(title).build();
        (row.clone().upcast(), row.upcast())
    }
}

pub fn present(
    app: &Rc<App>,
    id: &str,
    title: &str,
    prompt: Prompt,
    username: String,
    splitter: Option<StaticChallengePassword>,
    completion: Completion,
) {
    dismiss(app, id);
    if let Prompt::WebAuth { url } = &prompt {
        gtk::UriLauncher::new(url).launch(Some(&app.parent()), gtk::gio::Cancellable::NONE, |_| {});
        return;
    }
    let dialog = adw::AlertDialog::new(Some(&heading(&prompt)), Some(&body(title, &prompt, splitter.as_ref())));
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let mut fields = Fields {
        username: None,
        password: None,
        response: None,
        choice: None,
        remember: None,
    };
    let splits = splitter.as_ref().is_some_and(|s| !s.delimiter.is_empty());
    match &prompt {
        Prompt::Credentials { static_challenge, .. } => {
            let user = adw::EntryRow::builder().title("Username").text(&username).build();
            let pass = adw::PasswordEntryRow::builder().title("Password").build();
            list.append(&user);
            list.append(&pass);
            if let Some(sc) = static_challenge.as_ref().filter(|_| !splits) {
                let (row, editable) = response_row(&sc.text, sc.echo);
                list.append(&row);
                fields.response = Some(editable);
            }
            fields.username = Some(user);
            fields.password = Some(pass);
        }
        Prompt::Password { .. } => {
            let pass = adw::PasswordEntryRow::builder().title("Password").build();
            list.append(&pass);
            fields.password = Some(pass);
        }
        Prompt::Challenge { echo, .. } => {
            let (row, editable) = response_row("Response", *echo);
            list.append(&row);
            fields.response = Some(editable);
        }
        Prompt::NeedStr { .. } => {
            let (row, editable) = response_row("Value", true);
            list.append(&row);
            fields.response = Some(editable);
        }
        Prompt::Choice { options, .. } if !options.is_empty() => {
            let labels: Vec<&str> = options.iter().map(|o| o.label.as_str()).collect();
            let combo = adw::ComboRow::builder()
                .title("Certificate")
                .model(&gtk::StringList::new(&labels))
                .build();
            list.append(&combo);
            fields.choice = Some((combo, options.iter().map(|o| o.value.clone()).collect()));
        }
        _ => {}
    }
    if matches!(prompt, Prompt::Credentials { .. } | Prompt::Password { .. }) {
        let remember = adw::SwitchRow::builder().title("Save Password").build();
        list.append(&remember);
        fields.remember = Some(remember);
    }
    if list.first_child().is_some() {
        dialog.set_extra_child(Some(&list));
    }
    dialog.add_response("cancel", "Cancel");
    if matches!(prompt, Prompt::NeedOk { .. }) {
        dialog.add_response("decline", "Decline");
        dialog.add_response("ok", "Accept");
    } else if matches!(&prompt, Prompt::Choice { options, .. } if options.is_empty()) {
    } else {
        dialog.add_response("ok", "Connect");
    }
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");
    for editable in [
        fields.username.clone().map(|e| e.upcast::<gtk::Editable>()),
        fields.password.clone().map(|e| e.upcast()),
        fields.response.clone(),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(row) = editable.dynamic_cast_ref::<adw::EntryRow>() {
            row.set_activates_default(true);
        }
    }
    if let Some(user) = &fields.username {
        let d = dialog.clone();
        let update = move |e: &adw::EntryRow| d.set_response_enabled("ok", !e.text().is_empty());
        update(user);
        user.connect_changed(update);
    }

    let fields = Rc::new(fields);
    let completion = Rc::new(RefCell::new(Some(completion)));
    let app_weak = Rc::downgrade(app);
    let id_owned = id.to_string();
    dialog.connect_response(None, move |dialog, response| {
        if let Some(app) = app_weak.upgrade()
            && let Ok(mut prompts) = app.prompts.try_borrow_mut()
            && prompts.get(&id_owned).is_some_and(|d| d == dialog)
        {
            prompts.remove(&id_owned);
        }
        if response == "dismissed" {
            return;
        }
        let Some(done) = completion.borrow_mut().take() else {
            return;
        };
        let ok = response == "ok";
        if response == "cancel" {
            done(Reply::Cancel, false);
            return;
        }
        let text = |e: Option<&gtk::Editable>| e.map(|e| e.text().to_string()).unwrap_or_default();
        let remember = fields.remember.as_ref().is_some_and(|r| r.is_active());
        let password = text(fields.password.as_ref().map(|p| p.upcast_ref()));
        let response_text = text(fields.response.as_ref());
        let reply = match &prompt {
            Prompt::Credentials {
                realm,
                static_challenge,
                ..
            } => {
                let mut secret = password.clone();
                let mut challenge = static_challenge.as_ref().map(|_| response_text.clone());
                if static_challenge.is_some()
                    && let Some(s) = splitter.as_ref().filter(|s| !s.delimiter.is_empty())
                    && let Some(i) = password.rfind(&s.delimiter)
                {
                    secret = password[..i].to_string();
                    challenge = Some(password[i + s.delimiter.len()..].to_string());
                }
                Reply::Credentials {
                    realm: realm.clone(),
                    username: text(fields.username.as_ref().map(|u| u.upcast_ref())),
                    password: secret,
                    challenge_response: challenge,
                }
            }
            Prompt::Password { realm } => Reply::Password {
                realm: realm.clone(),
                password,
            },
            Prompt::Challenge { realm, .. } => Reply::Password {
                realm: realm.clone(),
                password: response_text,
            },
            Prompt::NeedOk { name, .. } => Reply::NeedOk { name: name.clone(), ok },
            Prompt::NeedStr { name, .. } => Reply::NeedStr {
                name: name.clone(),
                value: response_text,
            },
            Prompt::Choice { name, .. } => Reply::NeedStr {
                name: name.clone(),
                value: fields
                    .choice
                    .as_ref()
                    .and_then(|(combo, values)| values.get(combo.selected() as usize).cloned())
                    .unwrap_or_default(),
            },
            _ => Reply::Cancel,
        };
        done(reply, remember);
    });
    app.prompts.borrow_mut().insert(id.to_string(), dialog.clone());
    let window = app.parent();
    window.present();
    dialog.present(Some(&window));
    if let Some(first) = fields_focus(&dialog) {
        first.grab_focus();
    }
}

fn fields_focus(dialog: &adw::AlertDialog) -> Option<gtk::Widget> {
    let list = dialog.extra_child()?;
    let mut child = list.first_child();
    while let Some(row) = child {
        if let Some(entry) = row.dynamic_cast_ref::<adw::EntryRow>()
            && entry.text().is_empty()
        {
            return Some(row);
        }
        if row.is::<adw::PasswordEntryRow>() {
            return Some(row);
        }
        child = row.next_sibling();
    }
    None
}
