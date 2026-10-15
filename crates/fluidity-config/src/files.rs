use crate::document::{Document, Item};

const INLINE_MARKER: &str = "[inline]";

pub fn path_arg_index(name: &str, args: &[String]) -> Option<usize> {
    let index = match name {
        "ca"
        | "cert"
        | "key"
        | "dh"
        | "pkcs12"
        | "extra-certs"
        | "tls-auth"
        | "tls-crypt"
        | "tls-crypt-v2"
        | "secret"
        | "auth-user-pass"
        | "askpass"
        | "http-proxy-user-pass" => 0,
        "crl-verify" if args.get(1).is_some_and(|a| a == "dir") => return None,
        "crl-verify" => 0,
        "http-proxy" => 2,
        _ => return None,
    };
    let arg = args.get(index)?;
    (arg != INLINE_MARKER && !arg.is_empty() && !is_reserved_value(name, arg)).then_some(index)
}

fn is_reserved_value(name: &str, arg: &str) -> bool {
    name == "http-proxy" && matches!(arg, "auto" | "auto-nct" | "basic" | "ntlm" | "ntlm2" | "stdin")
        || name == "auth-user-pass" && arg == "stdin"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    pub item: usize,
    pub arg: usize,
    pub path: String,
}

pub fn file_refs(doc: &Document) -> Vec<FileRef> {
    doc.items
        .iter()
        .enumerate()
        .filter_map(|(item, it)| match it {
            Item::Directive { name, args } => {
                let arg = path_arg_index(name, args)?;
                Some(FileRef {
                    item,
                    arg,
                    path: args[arg].clone(),
                })
            }
            _ => None,
        })
        .collect()
}

pub fn set_ref(doc: &mut Document, file_ref: &FileRef, path: &str) {
    if let Some(Item::Directive { args, .. }) = doc.items.get_mut(file_ref.item) {
        args[file_ref.arg] = path.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_paths() {
        let doc = Document::parse(
            "ca ca.crt\ntls-auth \"my ta.key\" 1\nauth-user-pass\nhttp-proxy p 8080 creds.txt basic\nhttp-proxy p 8080 auto\ncrl-verify crl dir\ntls-crypt [inline]\n",
        )
        .unwrap();
        let paths: Vec<_> = file_refs(&doc).into_iter().map(|r| r.path).collect();
        assert_eq!(paths, ["ca.crt", "my ta.key", "creds.txt"]);
    }
}
