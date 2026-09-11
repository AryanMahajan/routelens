//! Shell word splitting — POSIX, and Windows `cmd.exe`.
//!
//! cURL commands are pasted from browser devtools, terminal history, and documentation, and
//! they arrive full of quoting: single quotes around JSON, `$'...'` escapes, backslash line
//! continuations, embedded newlines. Splitting on whitespace mangles all of it, so this runs
//! first and flag parsing works on real words.
//!
//! Chrome on Windows offers "Copy as cURL (cmd)", which escapes for `cmd.exe` instead:
//! `^"` for a quote, `^%` for a percent sign, `^` before a newline to continue, and `^\^"`
//! for a literal quote inside a quoted argument. That dialect is detected and handled by
//! [`tokenize`] too, since nobody should have to know which one they copied.

use std::iter::Peekable;
use std::str::Chars;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShellError {
    #[error("unterminated {} quote", match .quote { '\'' => "single", '"' => "double", _ => "" })]
    UnterminatedQuote { quote: char },
}

/// Split a command line into words, in whichever shell's quoting it was written for.
pub fn tokenize(input: &str) -> Result<Vec<String>, ShellError> {
    if looks_like_cmd(input) {
        return tokenize_cmd(input);
    }
    tokenize_posix(input)
}

/// `cmd.exe` escaping is unmistakable: `^"` never appears in a POSIX command line, where a
/// caret is an ordinary character and would sit inside quotes rather than before them.
fn looks_like_cmd(input: &str) -> bool {
    input.contains("^\"")
}

/// Split a `cmd.exe` command line.
///
/// Two layers, applied the way Windows does: `cmd.exe` strips carets (`^x` → `x`, and a
/// caret before a newline continues the line), then the C runtime splits what is left,
/// where `"` toggles quoting and `\"` is a literal quote. Single quotes mean nothing.
fn tokenize_cmd(input: &str) -> Result<Vec<String>, ShellError> {
    // Layer one: the caret.
    let mut unescaped = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '^' {
            unescaped.push(c);
            continue;
        }
        match chars.next() {
            Some('\r') => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                unescaped.push(' ');
            }
            Some('\n') => unescaped.push(' '),
            Some(next) => unescaped.push(next),
            None => unescaped.push('^'),
        }
    }

    // Layer two: the C runtime's argv rules.
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = unescaped.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\r' | '\n' => {
                if in_word {
                    tokens.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') if chars.peek() == Some(&'"') => {
                            chars.next();
                            current.push('"');
                        }
                        Some(ch) => current.push(ch),
                        None => return Err(ShellError::UnterminatedQuote { quote: '"' }),
                    }
                }
            }
            '\\' if chars.peek() == Some(&'"') => {
                chars.next();
                current.push('"');
                in_word = true;
            }
            _ => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        tokens.push(current);
    }
    Ok(tokens)
}

/// Split a POSIX shell command line.
fn tokenize_posix(input: &str) -> Result<Vec<String>, ShellError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    // Tracks whether a word has *started*, which is how `''` survives as an empty argument
    // rather than vanishing.
    let mut in_word = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\r' | '\n' => {
                if in_word {
                    tokens.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }

            // A backslash-newline is a line continuation and disappears; anything else after
            // a backslash is taken literally.
            '\\' => match chars.next() {
                Some('\n') => {}
                Some('\r') => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                }
                Some(next) => {
                    current.push(next);
                    in_word = true;
                }
                None => {
                    current.push('\\');
                    in_word = true;
                }
            },

            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => current.push(ch),
                        None => return Err(ShellError::UnterminatedQuote { quote: '\'' }),
                    }
                }
            }

            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            // Only these four are special inside double quotes; every other
                            // backslash stays literal, which is what keeps a Windows path or
                            // a JSON escape intact.
                            Some(esc @ ('"' | '\\' | '$' | '`')) => current.push(esc),
                            Some('\n') => {}
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => return Err(ShellError::UnterminatedQuote { quote: '"' }),
                        },
                        Some(ch) => current.push(ch),
                        None => return Err(ShellError::UnterminatedQuote { quote: '"' }),
                    }
                }
            }

            // `$'...'` — ANSI-C quoting. Chrome's "Copy as cURL" emits this whenever a header
            // or body contains a character it would rather escape than include literally.
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some('\\') => push_ansi_escape(&mut chars, &mut current),
                        Some(ch) => current.push(ch),
                        None => return Err(ShellError::UnterminatedQuote { quote: '\'' }),
                    }
                }
            }

            // A `#` only starts a comment at the beginning of a word, so a URL fragment
            // survives.
            '#' if !in_word => {
                for ch in chars.by_ref() {
                    if ch == '\n' {
                        break;
                    }
                }
            }

            _ => {
                current.push(c);
                in_word = true;
            }
        }
    }

    if in_word {
        tokens.push(current);
    }

    Ok(tokens)
}

fn push_ansi_escape(chars: &mut Peekable<Chars<'_>>, out: &mut String) {
    let Some(esc) = chars.next() else {
        out.push('\\');
        return;
    };

    match esc {
        'n' => out.push('\n'),
        't' => out.push('\t'),
        'r' => out.push('\r'),
        'a' => out.push('\u{7}'),
        'b' => out.push('\u{8}'),
        'f' => out.push('\u{c}'),
        'v' => out.push('\u{b}'),
        'e' => out.push('\u{1b}'),
        '0' => out.push('\0'),
        '\\' => out.push('\\'),
        '\'' => out.push('\''),
        '"' => out.push('"'),
        'x' => {
            let mut hex = String::new();
            while hex.len() < 2 {
                match chars.peek() {
                    Some(c) if c.is_ascii_hexdigit() => {
                        hex.push(*c);
                        chars.next();
                    }
                    _ => break,
                }
            }
            match u8::from_str_radix(&hex, 16) {
                Ok(byte) => out.push(byte as char),
                Err(_) => {
                    out.push('\\');
                    out.push('x');
                    out.push_str(&hex);
                }
            }
        }
        'u' => {
            let mut hex = String::new();
            while hex.len() < 4 {
                match chars.peek() {
                    Some(c) if c.is_ascii_hexdigit() => {
                        hex.push(*c);
                        chars.next();
                    }
                    _ => break,
                }
            }
            match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                Some(ch) => out.push(ch),
                None => {
                    out.push('\\');
                    out.push('u');
                    out.push_str(&hex);
                }
            }
        }
        other => {
            out.push('\\');
            out.push(other);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chrome's "Copy as cURL (cmd)" on Windows, trimmed. Every escape it uses is here:
    /// `^"` quotes, `^%` in the cookie, `^$` in the body, `^\^"` for quotes inside a
    /// header, and `^` line continuations.
    #[test]
    fn cmd_dialect_from_chrome_on_windows() {
        let input = "curl --url ^\"https://example.com/a^\" ^\r\n  -H ^\"accept: */*^\" ^\r\n  -b ^\"k=^%^7B^%^22x^%^22^%^7D^\" ^\r\n  -H ^\"sec-ch-ua: ^\\^\"Chromium^\\^\";v=^\\^\"152^\\^\"^\" ^\r\n  --data-raw ^\"a^$b+c^\"";
        let tokens = tokenize(input).unwrap();
        assert_eq!(
            tokens,
            vec![
                "curl",
                "--url",
                "https://example.com/a",
                "-H",
                "accept: */*",
                "-b",
                "k=%7B%22x%22%7D",
                "-H",
                "sec-ch-ua: \"Chromium\";v=\"152\"",
                "--data-raw",
                "a$b+c",
            ]
        );
    }

    #[test]
    fn a_caret_in_a_posix_command_is_just_a_character() {
        assert_eq!(tokenize("curl 'a^b'").unwrap(), vec!["curl", "a^b"]);
        assert_eq!(tokenize("curl a^b").unwrap(), vec!["curl", "a^b"]);
    }

    fn t(input: &str) -> Vec<String> {
        tokenize(input).unwrap()
    }

    #[test]
    fn splits_on_whitespace() {
        assert_eq!(t("curl https://x.test"), vec!["curl", "https://x.test"]);
    }

    #[test]
    fn collapses_runs_of_whitespace() {
        assert_eq!(t("  a   \t b  "), vec!["a", "b"]);
    }

    #[test]
    fn single_quotes_are_literal() {
        assert_eq!(
            t(r#"-d '{"name":"Aryan"}'"#),
            vec!["-d", r#"{"name":"Aryan"}"#]
        );
    }

    #[test]
    fn a_backslash_inside_single_quotes_stays_put() {
        assert_eq!(t(r"'a\nb'"), vec![r"a\nb"]);
    }

    #[test]
    fn double_quotes_honour_only_the_four_special_escapes() {
        assert_eq!(t(r#""a\"b""#), vec![r#"a"b"#]);
        assert_eq!(t(r#""a\\b""#), vec![r"a\b"]);
        // A JSON escape survives, because \n is not special to the shell in double quotes.
        assert_eq!(t(r#""{\"a\": \"x\ny\"}""#), vec!["{\"a\": \"x\\ny\"}"]);
    }

    #[test]
    fn line_continuations_join_the_command() {
        let input = "curl https://x.test \\\n  -H 'Accept: application/json'";
        assert_eq!(
            t(input),
            vec!["curl", "https://x.test", "-H", "Accept: application/json"]
        );
    }

    #[test]
    fn handles_windows_line_endings() {
        assert_eq!(
            t("curl \\\r\n  https://x.test"),
            vec!["curl", "https://x.test"]
        );
    }

    #[test]
    fn ansi_c_quoting_decodes_escapes() {
        assert_eq!(t(r"$'line1\nline2'"), vec!["line1\nline2"]);
        assert_eq!(t(r"$'tab\there'"), vec!["tab\there"]);
        assert_eq!(t(r"$'it\'s'"), vec!["it's"]);
    }

    #[test]
    fn ansi_c_hex_and_unicode_escapes_decode() {
        assert_eq!(t(r"$'\x41\x42'"), vec!["AB"]);
        assert_eq!(t(r"$'✓'"), vec!["✓"]);
    }

    #[test]
    fn adjacent_quoted_and_bare_text_form_one_word() {
        assert_eq!(
            t(r#"Content-Type:" application/json""#),
            vec!["Content-Type: application/json"]
        );
        assert_eq!(t(r#"'a'"b"c"#), vec!["abc"]);
    }

    #[test]
    fn an_empty_quoted_string_is_a_real_argument() {
        assert_eq!(t("-d ''"), vec!["-d", ""]);
    }

    #[test]
    fn a_url_fragment_is_not_a_comment() {
        assert_eq!(
            t("curl https://x.test/page#section"),
            vec!["curl", "https://x.test/page#section"]
        );
    }

    #[test]
    fn a_leading_hash_starts_a_comment() {
        assert_eq!(
            t("curl https://x.test # trailing note"),
            vec!["curl", "https://x.test"]
        );
    }

    #[test]
    fn unterminated_quotes_are_reported() {
        assert_eq!(
            tokenize("curl 'https://x.test"),
            Err(ShellError::UnterminatedQuote { quote: '\'' })
        );
        assert_eq!(
            tokenize(r#"curl "https://x.test"#),
            Err(ShellError::UnterminatedQuote { quote: '"' })
        );
    }

    #[test]
    fn a_realistic_devtools_paste_survives() {
        let input = r#"curl 'https://api.example.com/users?page=2' \
  -H 'Authorization: Bearer token' \
  -H 'Content-Type: application/json' \
  --data-raw '{"name":"Aryan"}'"#;

        assert_eq!(
            t(input),
            vec![
                "curl",
                "https://api.example.com/users?page=2",
                "-H",
                "Authorization: Bearer token",
                "-H",
                "Content-Type: application/json",
                "--data-raw",
                r#"{"name":"Aryan"}"#,
            ]
        );
    }
}
