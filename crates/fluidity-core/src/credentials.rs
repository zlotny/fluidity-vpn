const SERVICE: &str = "Fluidity VPN";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Credentials {
    pub username: Option<String>,
    pub password: Option<String>,
}

pub struct CredentialStore;

const PROXY_REALMS: [&str; 2] = ["http-proxy", "socks-proxy"];

fn entry(scope: &str, field: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(SERVICE, &format!("{scope}:{field}")).ok()
}

impl CredentialStore {
    pub fn load(scope: &str) -> Credentials {
        let get = |field| entry(scope, field).and_then(|e| e.get_password().ok());
        Credentials {
            username: get("username"),
            password: get("password"),
        }
    }

    pub fn save(scope: &str, credentials: &Credentials) -> Result<(), String> {
        for (field, value) in [("username", &credentials.username), ("password", &credentials.password)] {
            let Some(e) = entry(scope, field) else { continue };
            match value {
                Some(v) => e.set_password(v).map_err(|err| err.to_string())?,
                None => {
                    let _ = e.delete_credential();
                }
            }
        }
        Ok(())
    }

    pub fn save_secret(scope: &str, field: &str, value: &str) -> Result<(), String> {
        entry(scope, field)
            .ok_or("credential store unavailable")?
            .set_password(value)
            .map_err(|e| e.to_string())
    }

    pub fn secret(scope: &str, field: &str) -> Option<String> {
        entry(scope, field)?.get_password().ok()
    }

    pub fn clear(scope: &str) {
        let proxies = PROXY_REALMS.map(|realm| format!("{scope}:{realm}"));
        let fields = ["username", "password", "private-key"]
            .into_iter()
            .map(|field| (scope, field))
            .chain(
                proxies
                    .iter()
                    .flat_map(|p| [(p.as_str(), "username"), (p.as_str(), "password")]),
            );
        for (scope, field) in fields {
            if let Some(e) = entry(scope, field) {
                let _ = e.delete_credential();
            }
        }
    }
}
