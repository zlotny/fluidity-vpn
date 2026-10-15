use std::fmt;

use crate::Error;
use crate::lexer::{join, tokenize};

pub const META_PREFIX: &str = "#fluidity";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Directive { name: String, args: Vec<String> },
    Meta { key: String, args: Vec<String> },
    Inline { tag: String, body: String },
    Comment(String),
    Blank,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub items: Vec<Item>,
}

fn meta_rest(line: &str) -> Option<&str> {
    let rest = line.strip_prefix(META_PREFIX)?;
    rest.starts_with(char::is_whitespace).then_some(rest)
}

fn inline_open(line: &str) -> Option<&str> {
    let tag = line.strip_prefix('<')?.strip_suffix('>')?;
    let valid = !tag.is_empty()
        && !tag.starts_with('/')
        && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then_some(tag)
}

impl Document {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let mut items = Vec::new();
        let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l));

        while let Some((line_no, raw)) = lines.next() {
            let line = raw.trim();
            if line.is_empty() {
                items.push(Item::Blank);
                continue;
            }
            if let Some(rest) = meta_rest(line) {
                let mut tokens = tokenize(rest, line_no)?;
                if tokens.is_empty() {
                    items.push(Item::Comment(line.to_string()));
                    continue;
                }
                let key = tokens.remove(0);
                items.push(Item::Meta { key, args: tokens });
                continue;
            }
            if line.starts_with(['#', ';']) {
                items.push(Item::Comment(line.to_string()));
                continue;
            }
            if let Some(tag) = inline_open(line) {
                let close = format!("</{tag}>");
                let mut body = String::new();
                let mut closed = false;
                for (_, inner) in lines.by_ref() {
                    if inner.trim() == close {
                        closed = true;
                        break;
                    }
                    body.push_str(inner);
                    body.push('\n');
                }
                if !closed {
                    return Err(Error::UnterminatedInline {
                        line: line_no,
                        tag: tag.to_string(),
                    });
                }
                items.push(Item::Inline {
                    tag: tag.to_string(),
                    body,
                });
                continue;
            }
            let mut tokens = tokenize(line, line_no)?;
            if tokens.is_empty() {
                items.push(Item::Blank);
                continue;
            }
            let first = tokens.remove(0);
            let name = first.strip_prefix("--").unwrap_or(&first).to_string();
            items.push(Item::Directive { name, args: tokens });
        }

        Ok(Self { items })
    }

    pub fn directives<'a, 'n>(&'a self, name: &'n str) -> impl Iterator<Item = &'a [String]> + use<'a, 'n> {
        self.items.iter().filter_map(move |item| match item {
            Item::Directive { name: n, args } if n == name => Some(args.as_slice()),
            _ => None,
        })
    }

    pub fn directive(&self, name: &str) -> Option<&[String]> {
        self.directives(name).next()
    }

    pub fn has(&self, name: &str) -> bool {
        self.directive(name).is_some() || self.inline(name).is_some()
    }

    pub fn directive_names(&self) -> impl Iterator<Item = &str> {
        self.items.iter().filter_map(|item| match item {
            Item::Directive { name, .. } => Some(name.as_str()),
            Item::Inline { tag, .. } => Some(tag.as_str()),
            _ => None,
        })
    }

    pub fn set(&mut self, name: &str, args: Vec<String>) {
        let new = Item::Directive {
            name: name.to_string(),
            args,
        };
        self.replace_all(
            |item| matches!(item, Item::Directive { name: n, .. } if n == name),
            vec![new],
        );
    }

    pub fn push(&mut self, name: &str, args: Vec<String>) {
        self.items.push(Item::Directive {
            name: name.to_string(),
            args,
        });
    }

    pub fn remove(&mut self, name: &str) {
        self.items
            .retain(|item| !matches!(item, Item::Directive { name: n, .. } if n == name));
    }

    pub fn metas<'a, 'k>(&'a self, key: &'k str) -> impl Iterator<Item = &'a [String]> + use<'a, 'k> {
        self.items.iter().filter_map(move |item| match item {
            Item::Meta { key: k, args } if k == key => Some(args.as_slice()),
            _ => None,
        })
    }

    pub fn meta(&self, key: &str) -> Option<&str> {
        self.metas(key).next().and_then(|args| args.first()).map(String::as_str)
    }

    pub fn meta_values(&self, key: &str) -> Vec<String> {
        self.metas(key).flat_map(|args| args.iter().cloned()).collect()
    }

    pub fn meta_bool(&self, key: &str) -> Option<bool> {
        self.meta(key).map(|v| v.eq_ignore_ascii_case("true"))
    }

    pub fn set_meta(&mut self, key: &str, values: &[&str]) {
        let new = values
            .iter()
            .map(|v| Item::Meta {
                key: key.to_string(),
                args: vec![v.to_string()],
            })
            .collect();
        self.replace_all(|item| matches!(item, Item::Meta { key: k, .. } if k == key), new);
    }

    pub fn inline(&self, tag: &str) -> Option<&str> {
        self.items.iter().find_map(|item| match item {
            Item::Inline { tag: t, body } if t == tag => Some(body.as_str()),
            _ => None,
        })
    }

    pub fn set_inline(&mut self, tag: &str, body: &str) {
        let mut body = body.to_string();
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        let new = Item::Inline {
            tag: tag.to_string(),
            body,
        };
        self.replace_all(
            |item| matches!(item, Item::Inline { tag: t, .. } if t == tag),
            vec![new],
        );
    }

    fn replace_all(&mut self, matches: impl Fn(&Item) -> bool, new: Vec<Item>) {
        let position = self.items.iter().position(&matches);
        self.items.retain(|item| !matches(item));
        match position {
            Some(p) => {
                let p = p.min(self.items.len());
                self.items.splice(p..p, new);
            }
            None => self.items.extend(new),
        }
    }
}

impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for item in &self.items {
            match item {
                Item::Directive { name, args } if args.is_empty() => writeln!(f, "{name}")?,
                Item::Directive { name, args } => writeln!(f, "{name} {}", join(args))?,
                Item::Meta { key, args } if args.is_empty() => writeln!(f, "{META_PREFIX} {key}")?,
                Item::Meta { key, args } => writeln!(f, "{META_PREFIX} {key} {}", join(args))?,
                Item::Inline { tag, body } => write!(f, "<{tag}>\n{body}</{tag}>\n")?,
                Item::Comment(text) => writeln!(f, "{text}")?,
                Item::Blank => writeln!(f)?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"#fluidity startonopen false
#fluidity dnsserver 10.0.0.1
#fluidity dnsserver 10.0.0.2
#fluidity name "Office VPN"
--client
remote vpn.example.com 1194 udp
# a comment
<ca>
-----BEGIN CERTIFICATE-----
MIIB
-----END CERTIFICATE-----
</ca>

tls-auth ta.key 1
"#;

    #[test]
    fn parses_items() {
        let doc = Document::parse(SAMPLE).unwrap();
        assert_eq!(doc.meta("name"), Some("Office VPN"));
        assert_eq!(doc.meta_bool("startonopen"), Some(false));
        assert_eq!(doc.meta_values("dnsserver"), ["10.0.0.1", "10.0.0.2"]);
        assert!(doc.has("client"));
        assert_eq!(doc.directive("remote").unwrap(), ["vpn.example.com", "1194", "udp"]);
        assert!(doc.inline("ca").unwrap().contains("MIIB"));
        assert_eq!(doc.directive("tls-auth").unwrap(), ["ta.key", "1"]);
    }

    #[test]
    fn round_trips() {
        let doc = Document::parse(SAMPLE).unwrap();
        let again = Document::parse(&doc.to_string()).unwrap();
        assert_eq!(doc, again);
    }

    #[test]
    fn requires_space_after_prefix() {
        let doc = Document::parse(
            "#fluidityx ignored
",
        )
        .unwrap();
        assert!(matches!(doc.items[0], Item::Comment(_)));
    }

    #[test]
    fn edits_in_place() {
        let mut doc = Document::parse(SAMPLE).unwrap();
        doc.set_meta("dnsserver", &["1.1.1.1"]);
        assert_eq!(doc.meta_values("dnsserver"), ["1.1.1.1"]);
        assert!(matches!(&doc.items[1], Item::Meta { key, .. } if key == "dnsserver"));

        doc.set("remote", vec!["other.example.com".into()]);
        assert_eq!(doc.directives("remote").count(), 1);
        doc.remove("client");
        assert!(!doc.has("client"));
        doc.set_inline("ca", "NEW");
        assert_eq!(doc.inline("ca"), Some("NEW\n"));
    }

    #[test]
    fn reports_unterminated_inline() {
        let err = Document::parse("<ca>\nabc\n").unwrap_err();
        assert!(matches!(err, Error::UnterminatedInline { line: 1, .. }));
    }
}
