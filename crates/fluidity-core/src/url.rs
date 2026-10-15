#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlAction {
    Connect(Option<String>),
    Disconnect(Option<String>),
    Toggle(Option<String>),
}

pub fn parse_url(url: &str) -> Option<UrlAction> {
    let (scheme, rest) = url.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("fluidity") {
        return None;
    }
    let rest = rest.trim_start_matches('/');
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (action, name) = rest.split_once('/').unwrap_or((rest, ""));
    let name = decode(name.trim_end_matches('/'));
    let name = (!name.is_empty()).then_some(name);
    match action.to_ascii_lowercase().as_str() {
        "connect" => Some(UrlAction::Connect(name)),
        "disconnect" => Some(UrlAction::Disconnect(name)),
        "toggle" => Some(UrlAction::Toggle(name)),
        _ => None,
    }
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let pair = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], pair.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_actions() {
        assert_eq!(
            parse_url("fluidity://connect/Sydney%20Office"),
            Some(UrlAction::Connect(Some("Sydney Office".into())))
        );
        assert_eq!(parse_url("FLUIDITY://Disconnect"), Some(UrlAction::Disconnect(None)));
        assert_eq!(parse_url("fluidity:toggle/"), Some(UrlAction::Toggle(None)));
        assert_eq!(
            parse_url("fluidity://toggle/Caf%C3%A9%20VPN/?x=1"),
            Some(UrlAction::Toggle(Some("Café VPN".into())))
        );
        assert_eq!(parse_url("fluidity://delete/a"), None);
        assert_eq!(parse_url("https://connect/a"), None);
        assert_eq!(decode("100%"), "100%");
    }
}
