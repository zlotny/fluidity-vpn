use crate::document::{Document, Item};

const UNSAFE: &[&str] = &[
    "up",
    "down",
    "route-up",
    "route-pre-down",
    "ipchange",
    "tls-verify",
    "auth-user-pass-verify",
    "client-connect",
    "client-disconnect",
    "learn-address",
    "plugin",
    "iproute",
    "tls-export-cert",
    "log",
    "log-append",
    "writepid",
    "status",
    "replay-persist",
    "ifconfig-pool-persist",
    "tmp-dir",
    "cd",
    "chroot",
    "user",
    "group",
    "daemon",
    "config",
    "engine",
    "providers",
    "capath",
];

const RESERVED: &[&str] = &[
    "management",
    "management-client",
    "management-client-auth",
    "management-client-user",
    "management-client-group",
    "management-query-passwords",
    "management-query-proxy",
    "management-query-remote",
    "management-hold",
    "management-signal",
    "management-forget-disconnect",
    "management-up-down",
    "management-log-cache",
    "management-external-key",
    "management-external-cert",
    "dev-node",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    Unsafe(String),
    Reserved(String),
}

impl Violation {
    pub fn directive(&self) -> &str {
        match self {
            Self::Unsafe(name) | Self::Reserved(name) => name,
        }
    }
}

pub fn is_unsafe(name: &str, args: &[String]) -> bool {
    if name == "script-security" {
        return args
            .first()
            .and_then(|level| level.parse::<u8>().ok())
            .is_none_or(|level| level > 1);
    }
    UNSAFE.contains(&name)
}

pub fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&name)
}

pub fn check(doc: &Document) -> Vec<Violation> {
    doc.items
        .iter()
        .filter_map(|item| match item {
            Item::Directive { name, .. } if is_reserved(name) => Some(Violation::Reserved(name.clone())),
            Item::Directive { name, args } if is_unsafe(name, args) => Some(Violation::Unsafe(name.clone())),
            _ => None,
        })
        .collect()
}

pub fn foreign_files<'a>(doc: &'a Document, bundled: &[&str]) -> Vec<&'a str> {
    crate::files::file_refs(doc)
        .into_iter()
        .filter_map(|r| match &doc.items[r.item] {
            Item::Directive { args, .. } => args.get(r.arg).map(String::as_str),
            _ => None,
        })
        .filter(|path| !bundled.contains(path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_files_outside_bundle() {
        let doc = Document::parse("ca ca.crt\ncert /etc/master.passwd\nkey ../k.pem\n<tls-crypt>\nX\n</tls-crypt>\n").unwrap();
        assert_eq!(foreign_files(&doc, &["ca.crt"]), ["/etc/master.passwd", "../k.pem"]);
    }

    #[test]
    fn flags_script_and_file_writing_commands() {
        let doc = Document::parse(
            "client\nup /tmp/x.sh\nscript-security 2\nscript-security 1\nlog /etc/passwd\nmanagement 127.0.0.1 7505\n",
        )
        .unwrap();
        let names: Vec<_> = check(&doc).iter().map(|v| v.directive().to_string()).collect();
        assert_eq!(names, ["up", "script-security", "log", "management"]);
        assert!(matches!(check(&doc)[3], Violation::Reserved(_)));
    }

    #[test]
    fn accepts_plain_client_config() {
        let doc = Document::parse("client\nremote a 1194\nscript-security 0\nca ca.crt\n").unwrap();
        assert!(check(&doc).is_empty());
    }
}
