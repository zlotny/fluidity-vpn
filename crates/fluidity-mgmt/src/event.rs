use base64::Engine;
use base64::engine::general_purpose::STANDARD;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Connecting,
    Wait,
    Auth,
    AuthPending,
    GetConfig,
    AssignIp,
    AddRoutes,
    Connected,
    Reconnecting,
    Exiting,
    Resolve,
    TcpConnect,
    Unknown,
}

impl State {
    pub fn parse(s: &str) -> Self {
        match s {
            "CONNECTING" => Self::Connecting,
            "WAIT" => Self::Wait,
            "AUTH" => Self::Auth,
            "AUTH_PENDING" => Self::AuthPending,
            "GET_CONFIG" => Self::GetConfig,
            "ASSIGN_IP" => Self::AssignIp,
            "ADD_ROUTES" => Self::AddRoutes,
            "CONNECTED" => Self::Connected,
            "RECONNECTING" => Self::Reconnecting,
            "EXITING" => Self::Exiting,
            "RESOLVE" => Self::Resolve,
            "TCP_CONNECT" => Self::TcpConnect,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChange {
    pub time: u64,
    pub state: State,
    pub description: String,
    pub local_ip: String,
    pub remote_ip: String,
    pub remote_port: String,
    pub local_address: String,
    pub local_port: String,
    pub local_ipv6: String,
}

impl StateChange {
    pub fn parse(payload: &str) -> Self {
        let f: Vec<&str> = payload.split(',').collect();
        let field = |i: usize| f.get(i).copied().unwrap_or_default().to_string();
        Self {
            time: f.first().and_then(|t| t.parse().ok()).unwrap_or_default(),
            state: State::parse(f.get(1).copied().unwrap_or_default()),
            description: field(2),
            local_ip: field(3),
            remote_ip: field(4),
            remote_port: field(5),
            local_address: field(6),
            local_port: field(7),
            local_ipv6: field(8),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub time: u64,
    pub flags: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticChallenge {
    pub echo: bool,
    pub concat: bool,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicChallenge {
    pub echo: bool,
    pub response_required: bool,
    pub state_id: String,
    pub username: String,
    pub text: String,
}

impl DynamicChallenge {
    pub fn parse(s: &str) -> Option<Self> {
        let rest = s.strip_prefix("CRV1:")?;
        let mut parts = rest.splitn(4, ':');
        let flags = parts.next()?;
        let state_id = parts.next()?.to_string();
        let username_b64 = parts.next()?;
        let text = parts.next()?.to_string();
        let flags: Vec<&str> = flags.split(',').collect();
        let username = STANDARD
            .decode(username_b64)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();
        Some(Self {
            echo: flags.contains(&"E"),
            response_required: flags.contains(&"R"),
            state_id,
            username,
            text,
        })
    }

    pub fn response(&self, answer: &str) -> String {
        format!("CRV1::{}::{answer}", self.state_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    UsernamePassword,
    Password,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordRequest {
    Need {
        realm: String,
        need: Need,
        static_challenge: Option<StaticChallenge>,
    },
    VerificationFailed {
        realm: String,
        challenge: Option<DynamicChallenge>,
    },
    AuthToken(String),
}

impl PasswordRequest {
    pub fn parse(payload: &str) -> Option<Self> {
        if let Some(token) = payload.strip_prefix("Auth-Token:") {
            return Some(Self::AuthToken(token.to_string()));
        }
        if let Some(rest) = payload.strip_prefix("Verification Failed: ") {
            let realm = quoted(rest)?;
            let challenge = rest
                .split_once("['")
                .and_then(|(_, tail)| tail.rsplit_once("']"))
                .and_then(|(inner, _)| DynamicChallenge::parse(inner));
            return Some(Self::VerificationFailed { realm, challenge });
        }
        let rest = payload.strip_prefix("Need ")?;
        let realm = quoted(rest)?;
        let after = rest.split_once("' ").map(|(_, a)| a).unwrap_or_default();
        let need = if after.starts_with("username/password") {
            Need::UsernamePassword
        } else {
            Need::Password
        };
        let static_challenge = after.split_once("SC:").and_then(|(_, sc)| {
            let (flags, text) = sc.split_once(',')?;
            let flags: u32 = flags.parse().ok()?;
            Some(StaticChallenge {
                echo: flags & 1 != 0,
                concat: flags & 2 != 0,
                text: text.to_string(),
            })
        });
        Some(Self::Need {
            realm,
            need,
            static_challenge,
        })
    }
}

fn quoted(s: &str) -> Option<String> {
    let start = s.find('\'')? + 1;
    let len = s[start..].find('\'')?;
    Some(s[start..start + len].to_string())
}

fn need_message(payload: &str) -> Option<(String, String)> {
    let name = quoted(payload)?;
    let message = payload
        .split_once("MSG:")
        .map(|(_, m)| m.to_string())
        .unwrap_or_default();
    Some((name, message))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Info(String),
    State(StateChange),
    ByteCount {
        bytes_in: u64,
        bytes_out: u64,
    },
    Hold(String),
    Log(LogLine),
    Fatal(String),
    Password(PasswordRequest),
    NeedOk {
        name: String,
        message: String,
    },
    NeedStr {
        name: String,
        message: String,
    },
    Remote {
        host: String,
        port: String,
        proto: String,
    },
    Proxy {
        index: String,
        proto: String,
        host: String,
    },
    PkSign {
        data: String,
        algorithm: Option<String>,
    },
    NeedCertificate(String),
    InfoMsg(String),
    Echo {
        time: u64,
        param: String,
    },
    Notify(String),
    UpDown {
        direction: String,
        env: Vec<(String, String)>,
    },
    Unknown {
        kind: String,
        payload: String,
    },
}

impl Event {
    pub fn parse(kind: &str, payload: &str) -> Self {
        let unknown = || Self::Unknown {
            kind: kind.to_string(),
            payload: payload.to_string(),
        };
        match kind {
            "INFO" => Self::Info(payload.to_string()),
            "STATE" => Self::State(StateChange::parse(payload)),
            "BYTECOUNT" => {
                let (i, o) = payload.split_once(',').unwrap_or((payload, "0"));
                Self::ByteCount {
                    bytes_in: i.parse().unwrap_or_default(),
                    bytes_out: o.parse().unwrap_or_default(),
                }
            }
            "HOLD" => Self::Hold(payload.to_string()),
            "LOG" => {
                let mut parts = payload.splitn(3, ',');
                Self::Log(LogLine {
                    time: parts.next().and_then(|t| t.parse().ok()).unwrap_or_default(),
                    flags: parts.next().unwrap_or_default().to_string(),
                    message: parts.next().unwrap_or_default().to_string(),
                })
            }
            "FATAL" => Self::Fatal(payload.to_string()),
            "PASSWORD" => PasswordRequest::parse(payload)
                .map(Self::Password)
                .unwrap_or_else(unknown),
            "NEED-OK" => need_message(payload)
                .map(|(name, message)| Self::NeedOk { name, message })
                .unwrap_or_else(unknown),
            "NEED-STR" => need_message(payload)
                .map(|(name, message)| Self::NeedStr { name, message })
                .unwrap_or_else(unknown),
            "REMOTE" => {
                let mut p = payload.splitn(3, ',').map(str::to_string);
                Self::Remote {
                    host: p.next().unwrap_or_default(),
                    port: p.next().unwrap_or_default(),
                    proto: p.next().unwrap_or_default(),
                }
            }
            "PROXY" => {
                let mut p = payload.splitn(3, ',').map(str::to_string);
                Self::Proxy {
                    index: p.next().unwrap_or_default(),
                    proto: p.next().unwrap_or_default(),
                    host: p.next().unwrap_or_default(),
                }
            }
            "PK_SIGN" => {
                let (data, algorithm) = match payload.split_once(',') {
                    Some((d, a)) => (d.to_string(), Some(a.to_string())),
                    None => (payload.to_string(), None),
                };
                Self::PkSign { data, algorithm }
            }
            "NEED-CERTIFICATE" => Self::NeedCertificate(payload.to_string()),
            "INFOMSG" => Self::InfoMsg(payload.to_string()),
            "ECHO" => {
                let (time, param) = payload.split_once(',').unwrap_or(("0", payload));
                Self::Echo {
                    time: time.parse().unwrap_or_default(),
                    param: param.to_string(),
                }
            }
            "NOTIFY" => Self::Notify(payload.to_string()),
            _ => unknown(),
        }
    }

    pub fn web_auth_url(&self) -> Option<&str> {
        let Self::InfoMsg(msg) = self else { return None };
        msg.strip_prefix("OPEN_URL:").or_else(|| {
            msg.strip_prefix("WEB_AUTH:")
                .map(|r| r.split_once(':').map_or(r, |(_, url)| url))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_state() {
        let Event::State(s) = Event::parse("STATE", "1700000000,CONNECTED,SUCCESS,10.8.0.6,1.2.3.4,1194,,,fd00::6")
        else {
            panic!()
        };
        assert_eq!(s.state, State::Connected);
        assert_eq!(s.local_ip, "10.8.0.6");
        assert_eq!(s.remote_ip, "1.2.3.4");
        assert_eq!(s.local_ipv6, "fd00::6");
    }

    #[test]
    fn parses_password_requests() {
        assert_eq!(
            PasswordRequest::parse("Need 'Auth' username/password"),
            Some(PasswordRequest::Need {
                realm: "Auth".into(),
                need: Need::UsernamePassword,
                static_challenge: None
            })
        );
        assert_eq!(
            PasswordRequest::parse("Need 'Private Key' password"),
            Some(PasswordRequest::Need {
                realm: "Private Key".into(),
                need: Need::Password,
                static_challenge: None
            })
        );
        let Some(PasswordRequest::Need {
            static_challenge: Some(sc),
            ..
        }) = PasswordRequest::parse("Need 'Auth' username/password SC:3,Enter PIN, please")
        else {
            panic!()
        };
        assert!(sc.echo && sc.concat);
        assert_eq!(sc.text, "Enter PIN, please");
    }

    #[test]
    fn parses_dynamic_challenge() {
        let req = PasswordRequest::parse(
            "Verification Failed: 'Auth' ['CRV1:R,E:Om01u7Fh4LrGBS7uh0SWmzwabUiGiW6l:Y3Ix:Please enter token PIN']",
        )
        .unwrap();
        let PasswordRequest::VerificationFailed {
            realm,
            challenge: Some(c),
        } = req
        else {
            panic!()
        };
        assert_eq!(realm, "Auth");
        assert!(c.echo && c.response_required);
        assert_eq!(c.username, "cr1");
        assert_eq!(c.text, "Please enter token PIN");
        assert_eq!(c.response("123"), "CRV1::Om01u7Fh4LrGBS7uh0SWmzwabUiGiW6l::123");

        assert_eq!(
            PasswordRequest::parse("Verification Failed: 'Auth'"),
            Some(PasswordRequest::VerificationFailed {
                realm: "Auth".into(),
                challenge: None
            })
        );
    }

    #[test]
    fn parses_misc() {
        assert_eq!(
            Event::parse("BYTECOUNT", "100,200"),
            Event::ByteCount {
                bytes_in: 100,
                bytes_out: 200
            }
        );
        assert_eq!(
            Event::parse(
                "NEED-OK",
                "Need 'token-insertion-request' confirmation MSG:Please insert your token"
            ),
            Event::NeedOk {
                name: "token-insertion-request".into(),
                message: "Please insert your token".into()
            }
        );
        let Event::Log(l) = Event::parse("LOG", "1700000000,W,Warning, with comma") else {
            panic!()
        };
        assert_eq!(l.message, "Warning, with comma");
        assert_eq!(
            Event::parse("INFOMSG", "WEB_AUTH::https://sso.example.com/x").web_auth_url(),
            Some("https://sso.example.com/x")
        );
        assert_eq!(
            Event::parse("INFOMSG", "OPEN_URL:https://a/b").web_auth_url(),
            Some("https://a/b")
        );
    }
}
