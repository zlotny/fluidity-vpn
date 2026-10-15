use crate::Error;

pub fn tokenize(line: &str, line_no: usize) -> Result<Vec<String>, Error> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match quote {
            Some('\'') => {
                if c == '\'' {
                    quote = None;
                } else {
                    current.push(c);
                }
            }
            Some(_) => match c {
                '"' => quote = None,
                '\\' => match chars.next() {
                    Some(n) => current.push(n),
                    None => current.push('\\'),
                },
                _ => current.push(c),
            },
            None => {
                if c.is_whitespace() {
                    if in_token {
                        tokens.push(std::mem::take(&mut current));
                        in_token = false;
                    }
                    continue;
                }
                if !in_token && (c == '#' || c == ';') {
                    break;
                }
                in_token = true;
                match c {
                    '"' | '\'' => quote = Some(c),
                    '\\' => match chars.next() {
                        Some(n) => current.push(n),
                        None => current.push('\\'),
                    },
                    _ => current.push(c),
                }
            }
        }
    }

    if quote.is_some() {
        return Err(Error::UnterminatedQuote { line: line_no });
    }
    if in_token {
        tokens.push(current);
    }
    Ok(tokens)
}

pub fn quote(token: &str) -> String {
    let needs_quotes = token.is_empty()
        || token.starts_with(['#', ';'])
        || token
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'));
    if !needs_quotes {
        return token.to_string();
    }
    let mut out = String::with_capacity(token.len() + 2);
    out.push('"');
    for c in token.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

pub fn join(tokens: &[String]) -> String {
    tokens.iter().map(|t| quote(t)).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Vec<String> {
        tokenize(s, 1).unwrap()
    }

    #[test]
    fn splits_on_whitespace() {
        assert_eq!(
            t("  remote  vpn.example.com 1194 udp "),
            ["remote", "vpn.example.com", "1194", "udp"]
        );
    }

    #[test]
    fn handles_quotes_and_escapes() {
        assert_eq!(t(r#"name "Office main""#), ["name", "Office main"]);
        assert_eq!(t(r#"x 'a "b" c'"#), ["x", r#"a "b" c"#]);
        assert_eq!(t(r#"x "a \"b\" \\c""#), ["x", r#"a "b" \c"#]);
        assert_eq!(t(r"x a\ b"), ["x", "a b"]);
    }

    #[test]
    fn strips_trailing_comments() {
        assert_eq!(t("remote host # primary"), ["remote", "host"]);
        assert_eq!(t("remote host ; primary"), ["remote", "host"]);
        assert_eq!(t("remote ho#st"), ["remote", "ho#st"]);
        assert!(t("# only comment").is_empty());
    }

    #[test]
    fn rejects_unterminated_quote() {
        assert!(tokenize("x \"abc", 3).is_err());
    }

    #[test]
    fn quote_round_trips() {
        for s in ["plain", "with space", r#"q"uote"#, r"back\slash", "", "#hash", "it's"] {
            let line = format!("opt {}", quote(s));
            assert_eq!(t(&line), ["opt", s]);
        }
    }
}
