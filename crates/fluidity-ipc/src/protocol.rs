use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WireBundle {
    pub config: String,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConnectOptions {
    #[serde(default)]
    pub openvpn_version: Option<String>,
    #[serde(default)]
    pub auto_reconnect: bool,
    #[serde(default)]
    pub block_ipv6: bool,
    #[serde(default)]
    pub disable_time_machine: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    Hello {
        client: String,
    },
    List,
    Connect {
        id: String,
        bundle: WireBundle,
        #[serde(default)]
        options: ConnectOptions,
    },
    Disconnect {
        id: String,
    },
    DisconnectAll,
    Reply {
        id: String,
        reply: Reply,
    },
    Log {
        id: String,
    },
    Subscribe,
    SystemSettings,
    SetSystemSettings {
        settings: SystemSettings,
    },
    Versions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Credentials {
        realm: String,
        username: String,
        password: String,
        #[serde(default)]
        challenge_response: Option<String>,
    },
    Password {
        realm: String,
        password: String,
    },
    NeedOk {
        name: String,
        ok: bool,
    },
    NeedStr {
        name: String,
        value: String,
    },
    Certificate {
        pem: String,
    },
    Signature {
        signature: String,
    },
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Prompt {
    Credentials {
        realm: String,
        static_challenge: Option<StaticChallengePrompt>,
        #[serde(default)]
        retry: bool,
    },
    Password {
        realm: String,
    },
    Challenge {
        realm: String,
        text: String,
        echo: bool,
    },
    NeedOk {
        name: String,
        message: String,
    },
    NeedStr {
        name: String,
        message: String,
    },
    WebAuth {
        url: String,
    },
    Choice {
        name: String,
        message: String,
        options: Vec<ChoiceOption>,
    },
    Certificate {
        identity: String,
    },
    Sign {
        identity: String,
        data: String,
        algorithm: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticChallengePrompt {
    pub text: String,
    pub echo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    #[default]
    Disconnected,
    Connecting,
    Authenticating,
    Connected,
    Reconnecting,
    Disconnecting,
    Error,
}

impl SessionState {
    pub fn is_active(self) -> bool {
        !matches!(self, Self::Disconnected | Self::Error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub owner_uid: u32,
    pub state: SessionState,
    pub detail: String,
    pub local_ipv4: String,
    pub local_ipv6: String,
    pub remote_address: String,
    pub remote_port: String,
    pub interface: String,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub connected_since: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SystemSettings {
    pub allow_unsafe_commands: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub time: u64,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Session { session: SessionInfo },
    Removed { id: String },
    Prompt { id: String, prompt: Prompt },
    PromptDone { id: String },
    Log { id: String, entry: LogEntry },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Payload {
    Sessions(Vec<SessionInfo>),
    Log(Vec<LogEntry>),
    Settings(SystemSettings),
    Versions(Vec<String>),
    Hello { version: u32, daemon: String },
    Empty {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientMessage {
    pub seq: u64,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum ServerMessage {
    Ok { seq: u64, ok: Payload },
    Err { seq: u64, error: String },
    Event(Event),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let msg = ClientMessage {
            seq: 7,
            request: Request::Reply {
                id: "a".into(),
                reply: Reply::Credentials {
                    realm: "Auth".into(),
                    username: "u".into(),
                    password: "p".into(),
                    challenge_response: None,
                },
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""method":"reply""#));
        assert_eq!(serde_json::from_str::<ClientMessage>(&json).unwrap(), msg);

        let list = ClientMessage {
            seq: 1,
            request: Request::List,
        };
        let json = serde_json::to_string(&list).unwrap();
        assert_eq!(serde_json::from_str::<ClientMessage>(&json).unwrap(), list);

        for msg in [
            ServerMessage::Ok {
                seq: 1,
                ok: Payload::Empty {},
            },
            ServerMessage::Ok {
                seq: 2,
                ok: Payload::Sessions(vec![SessionInfo::default()]),
            },
            ServerMessage::Err {
                seq: 3,
                error: "nope".into(),
            },
            ServerMessage::Event(Event::Prompt {
                id: "x".into(),
                prompt: Prompt::WebAuth {
                    url: "https://a".into(),
                },
            }),
        ] {
            let json = serde_json::to_string(&msg).unwrap();
            assert_eq!(serde_json::from_str::<ServerMessage>(&json).unwrap(), msg, "{json}");
        }
    }
}
