//! cURL import.
//!
//! The rule this module exists to serve: you should never have to decide by hand whether a
//! pasted value belongs in headers, query, auth, or body.

use crate::error::{ImportError, Result};
use crate::shell::tokenize;
use crate::Imported;
use base64::Engine as _;
use rl_model::{AuthConfig, BodyValue, FormPart, HttpMethod, KeyValue, RequestDraft};
use std::path::PathBuf;

/// Parse a cURL command into a request.
pub fn parse_curl(input: &str) -> Result<Imported<RequestDraft>> {
    let tokens = tokenize(input).map_err(ImportError::Shell)?;
    if tokens.is_empty() {
        return Err(ImportError::Empty);
    }

    let mut state = Parsed::default();
    let mut warnings = Vec::new();
    let mut args = tokens.into_iter().peekable();

    // A pasted command usually starts with `curl`, but not always — someone may paste only
    // the arguments.
    if args.peek().map(|t| t.as_str()) == Some("curl") {
        args.next();
    }

    while let Some(arg) = args.next() {
        let mut take_value = |flag: &str| -> Result<String> {
            args.next().ok_or_else(|| ImportError::MissingValue {
                flag: flag.to_string(),
            })
        };

        match arg.as_str() {
            "-X" | "--request" => state.method = Some(take_value("-X")?),
            "-H" | "--header" => state.headers.push(take_value("-H")?),
            "--url" => state.url = Some(take_value("--url")?),

            "-d" | "--data" | "--data-raw" | "--data-ascii" | "--data-binary" => {
                state.data.push(take_value("-d")?)
            }
            "--data-urlencode" => {
                let raw = take_value("--data-urlencode")?;
                state.data.push(urlencode_data(&raw));
            }
            "--json" => {
                // curl >= 7.82: implies POST plus JSON content-type and accept headers.
                state.data.push(take_value("--json")?);
                state.headers.push("Content-Type: application/json".into());
                state.headers.push("Accept: application/json".into());
            }

            "-F" | "--form" => state.form.push(take_value("-F")?),
            "-u" | "--user" => state.user = Some(take_value("-u")?),
            "-b" | "--cookie" => state.cookies.push(take_value("-b")?),
            "-A" | "--user-agent" => {
                let ua = take_value("-A")?;
                state.headers.push(format!("User-Agent: {ua}"));
            }
            "-e" | "--referer" => {
                let referer = take_value("-e")?;
                state.headers.push(format!("Referer: {referer}"));
            }

            "-G" | "--get" => state.data_as_query = true,
            "-I" | "--head" => state.head = true,
            "-k" | "--insecure" => state.insecure = true,
            "-L" | "--location" => state.follow_redirects = true,
            "--compressed" => state.compressed = true,

            "-m" | "--max-time" => {
                let seconds = take_value("-m")?;
                if let Ok(s) = seconds.parse::<f64>() {
                    state.timeout_ms = Some((s * 1000.0) as u64);
                }
            }

            // Output and verbosity flags describe what curl does with the response, which is
            // RouteLens's job now. Dropping them silently is correct.
            "-s" | "--silent" | "-S" | "--show-error" | "-v" | "--verbose" | "-i" | "--include"
            | "-f" | "--fail" | "-O" | "--remote-name" | "-#" | "--progress-bar" | "-N"
            | "--no-buffer" | "-g" | "--globoff" => {}
            "-o" | "--output" | "--max-redirs" | "--connect-timeout" | "--retry" => {
                let _ = args.next();
            }

            other if other.starts_with('-') && other.len() > 1 => {
                warnings.push(format!("ignored unrecognised flag `{other}`"));
                // A flag taking a value would otherwise leave its value looking like a URL.
                if let Some(next) = args.peek() {
                    if !next.starts_with('-') && state.url.is_some() {
                        args.next();
                    }
                }
            }

            bare => {
                if state.url.is_none() {
                    state.url = Some(bare.to_string());
                } else {
                    warnings.push(format!("ignored extra argument `{bare}`"));
                }
            }
        }
    }

    state.into_draft(warnings)
}

#[derive(Debug, Default)]
struct Parsed {
    url: Option<String>,
    method: Option<String>,
    headers: Vec<String>,
    data: Vec<String>,
    form: Vec<String>,
    cookies: Vec<String>,
    user: Option<String>,
    data_as_query: bool,
    head: bool,
    insecure: bool,
    follow_redirects: bool,
    compressed: bool,
    timeout_ms: Option<u64>,
}

impl Parsed {
    fn into_draft(self, mut warnings: Vec<String>) -> Result<Imported<RequestDraft>> {
        let raw_url = self.url.ok_or(ImportError::NoUrl)?;
        let url = normalize_url(&raw_url);

        let body_present = !self.data.is_empty() || !self.form.is_empty();

        // curl's own precedence: an explicit -X wins, -I means HEAD, a body implies POST,
        // and -G forces the data into the query string leaving a GET behind.
        let method = match (&self.method, self.head, body_present, self.data_as_query) {
            (Some(m), ..) => m.parse::<HttpMethod>().unwrap_or(HttpMethod::Get),
            (None, true, ..) => HttpMethod::Head,
            (None, false, _, true) => HttpMethod::Get,
            (None, false, true, false) => HttpMethod::Post,
            _ => HttpMethod::Get,
        };

        let mut draft = RequestDraft::new(method, "");

        // Split the query out of the URL into structured rows, so an environment switch can
        // change one value without editing a string.
        let (base, query_rows) = crate::split_url_query(&url)?;
        draft.url = base;
        draft.query = query_rows;

        let joined_data = self.data.join("&");

        if self.data_as_query && !joined_data.is_empty() {
            for (key, value) in parse_pairs(&joined_data) {
                draft.query.push(KeyValue::new(key, value));
            }
        }

        // Headers, recognising the ones that are really something else.
        let mut content_type: Option<String> = None;
        for header in &self.headers {
            let Some((name, value)) = header.split_once(':') else {
                warnings.push(format!("ignored malformed header `{header}`"));
                continue;
            };
            let name = name.trim();
            let value = value.trim();

            // An unrecognised scheme falls through and stays a header rather than being lost.
            if name.eq_ignore_ascii_case("authorization") {
                if let Some(auth) = parse_authorization_header(value) {
                    draft.auth = auth;
                    continue;
                }
            }

            if name.eq_ignore_ascii_case("cookie") {
                for (key, val) in parse_cookies(value) {
                    draft.cookies.push(KeyValue::new(key, val));
                }
                continue;
            }

            if name.eq_ignore_ascii_case("content-type") {
                content_type = Some(value.to_string());
            }

            // curl adds these itself; carrying them over would pin the request to whatever
            // the browser negotiated at copy time.
            if name.eq_ignore_ascii_case("content-length")
                || name.eq_ignore_ascii_case("host")
                || (self.compressed && name.eq_ignore_ascii_case("accept-encoding"))
            {
                continue;
            }

            draft.headers.push(KeyValue::new(name, value));
        }

        for cookie in &self.cookies {
            for (key, val) in parse_cookies(cookie) {
                draft.cookies.push(KeyValue::new(key, val));
            }
        }

        if let Some(user) = &self.user {
            let (username, password) = user.split_once(':').unwrap_or((user.as_str(), ""));
            draft.auth = AuthConfig::Basic {
                username: username.to_string(),
                password: password.to_string(),
            };
        }

        // Body.
        if !self.form.is_empty() {
            draft.body = BodyValue::Multipart {
                parts: self.form.iter().map(|f| parse_form_part(f)).collect(),
            };
            // The boundary is generated at send time, so a copied one would be wrong.
            draft
                .headers
                .retain(|h| !h.key.eq_ignore_ascii_case("content-type"));
        } else if !joined_data.is_empty() && !self.data_as_query {
            draft.body = body_from_text(&joined_data, content_type.as_deref());
            // The content type now lives on the body.
            draft
                .headers
                .retain(|h| !h.key.eq_ignore_ascii_case("content-type"));
        }

        draft.settings.follow_redirects = self.follow_redirects;
        draft.settings.accept_invalid_certs = self.insecure;
        if let Some(ms) = self.timeout_ms {
            draft.settings.timeout_ms = ms;
        }

        if self.insecure {
            warnings.push(
                "`-k` disables certificate verification; RouteLens applies it to this request \
                 only and never saves it"
                    .into(),
            );
        }

        Ok(Imported {
            value: draft,
            warnings,
        })
    }
}

/// A URL pasted without a scheme is what curl itself assumes is HTTP.
fn normalize_url(raw: &str) -> String {
    if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{raw}")
    }
}

fn parse_pairs(data: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(data.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

pub(crate) fn parse_cookies(value: &str) -> Vec<(String, String)> {
    value
        .split(';')
        .filter_map(|pair| {
            let pair = pair.trim();
            if pair.is_empty() {
                return None;
            }
            let (k, v) = pair.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Turn an `Authorization` header into structured auth.
///
/// This is what lets an imported request round-trip: an environment can swap the token, and
/// export can re-render the header. A raw header string cannot be reasoned about.
pub(crate) fn parse_authorization_header(value: &str) -> Option<AuthConfig> {
    let (scheme, rest) = value.split_once(' ')?;

    if scheme.eq_ignore_ascii_case("bearer") {
        return Some(AuthConfig::Bearer {
            token: rest.trim().to_string(),
        });
    }

    if scheme.eq_ignore_ascii_case("basic") {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(rest.trim())
            .ok()?;
        let text = String::from_utf8(decoded).ok()?;
        let (username, password) = text.split_once(':')?;
        return Some(AuthConfig::Basic {
            username: username.to_string(),
            password: password.to_string(),
        });
    }

    None
}

pub(crate) fn body_from_text(data: &str, content_type: Option<&str>) -> BodyValue {
    let looks_json = serde_json::from_str::<serde_json::Value>(data).is_ok()
        && data.trim_start().starts_with(['{', '[']);

    match content_type {
        Some(ct) if ct.to_ascii_lowercase().contains("json") => BodyValue::Json {
            content: data.to_string(),
        },
        Some(ct) if ct.to_ascii_lowercase().contains("x-www-form-urlencoded") => BodyValue::Form {
            fields: parse_pairs(data)
                .into_iter()
                .map(|(k, v)| KeyValue::new(k, v))
                .collect(),
        },
        Some(ct) => BodyValue::Text {
            content: data.to_string(),
            content_type: ct.to_string(),
        },
        // No content type stated. curl would default to form encoding, but the pasted body
        // is usually JSON and treating it as such is what the user meant.
        None if looks_json => BodyValue::Json {
            content: data.to_string(),
        },
        None => BodyValue::Form {
            fields: parse_pairs(data)
                .into_iter()
                .map(|(k, v)| KeyValue::new(k, v))
                .collect(),
        },
    }
}

fn parse_form_part(raw: &str) -> FormPart {
    let (name, value) = raw.split_once('=').unwrap_or((raw, ""));

    if let Some(rest) = value.strip_prefix('@') {
        // `name=@path;type=image/png`
        let (path, content_type) = match rest.split_once(";type=") {
            Some((p, t)) => (p, Some(t.to_string())),
            None => (rest, None),
        };
        return FormPart::File {
            name: name.to_string(),
            path: PathBuf::from(path),
            content_type,
            enabled: true,
        };
    }

    FormPart::Text {
        name: name.to_string(),
        value: value.to_string(),
        enabled: true,
    }
}

fn urlencode_data(raw: &str) -> String {
    let encode = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();

    match raw.split_once('=') {
        Some((name, value)) if !name.is_empty() => format!("{}={}", name, encode(value)),
        Some((_, value)) => encode(value),
        None => encode(raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> RequestDraft {
        parse_curl(input).unwrap().value
    }

    #[test]
    fn the_documented_example_splits_into_every_field() {
        // Straight from docs/import.md.
        let draft = parse(
            r#"curl 'https://api.example.com/users?page=2' \
  -H 'Authorization: Bearer token' \
  -H 'Content-Type: application/json' \
  --data-raw '{"name":"Aryan"}'"#,
        );

        assert_eq!(draft.method, HttpMethod::Post);
        assert_eq!(draft.url, "https://api.example.com/users");
        assert_eq!(draft.query.len(), 1);
        assert_eq!(draft.query[0].key, "page");
        assert_eq!(draft.query[0].value, "2");
        assert_eq!(
            draft.auth,
            AuthConfig::Bearer {
                token: "token".into()
            }
        );
        match &draft.body {
            BodyValue::Json { content } => assert_eq!(content, r#"{"name":"Aryan"}"#),
            other => panic!("expected a json body, got {other:?}"),
        }
        // Content-Type moved onto the body rather than lingering as a header.
        assert!(draft.headers.iter().all(|h| h.key != "Content-Type"));
    }

    #[test]
    fn method_defaults_to_get() {
        assert_eq!(parse("curl https://x.test/").method, HttpMethod::Get);
    }

    #[test]
    fn a_body_implies_post() {
        assert_eq!(
            parse("curl https://x.test/ -d a=1").method,
            HttpMethod::Post
        );
    }

    #[test]
    fn an_explicit_method_wins_over_inference() {
        let draft = parse("curl -X PUT https://x.test/ -d a=1");
        assert_eq!(draft.method, HttpMethod::Put);
    }

    #[test]
    fn head_is_recognised() {
        assert_eq!(parse("curl -I https://x.test/").method, HttpMethod::Head);
    }

    #[test]
    fn an_unusual_method_is_preserved() {
        assert_eq!(
            parse("curl -X PURGE https://x.test/").method,
            HttpMethod::Other("PURGE".into())
        );
    }

    #[test]
    fn the_url_can_come_before_or_after_flags_or_via_the_url_flag() {
        assert_eq!(parse("curl https://x.test/a").url, "https://x.test/a");
        assert_eq!(parse("curl -s https://x.test/a").url, "https://x.test/a");
        assert_eq!(parse("curl --url https://x.test/a").url, "https://x.test/a");
    }

    #[test]
    fn a_scheme_less_url_becomes_http() {
        assert_eq!(
            parse("curl localhost:8000/health").url,
            "http://localhost:8000/health"
        );
    }

    #[test]
    fn basic_auth_arrives_from_the_user_flag_and_from_a_header() {
        let expected = AuthConfig::Basic {
            username: "aladdin".into(),
            password: "opensesame".into(),
        };
        assert_eq!(
            parse("curl -u aladdin:opensesame https://x.test/").auth,
            expected
        );
        assert_eq!(
            parse("curl https://x.test/ -H 'Authorization: Basic YWxhZGRpbjpvcGVuc2VzYW1l'").auth,
            expected
        );
    }

    #[test]
    fn an_unrecognised_auth_scheme_stays_a_header() {
        let draft = parse("curl https://x.test/ -H 'Authorization: Negotiate abc'");
        assert_eq!(draft.auth, AuthConfig::None);
        assert!(draft
            .headers
            .iter()
            .any(|h| h.key.eq_ignore_ascii_case("authorization")));
    }

    #[test]
    fn cookies_become_rows_from_either_source() {
        let draft = parse("curl https://x.test/ -b 'a=1; b=2'");
        assert_eq!(draft.cookies.len(), 2);

        let draft = parse("curl https://x.test/ -H 'Cookie: session=abc'");
        assert_eq!(draft.cookies[0].key, "session");
        assert_eq!(draft.cookies[0].value, "abc");
    }

    #[test]
    fn form_encoded_bodies_become_editable_rows() {
        let draft = parse("curl https://x.test/ -d 'name=Aryan&role=dev'");
        match &draft.body {
            BodyValue::Form { fields } => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].key, "name");
                assert_eq!(fields[1].value, "dev");
            }
            other => panic!("expected a form body, got {other:?}"),
        }
    }

    #[test]
    fn repeated_data_flags_concatenate_as_curl_does() {
        let draft = parse("curl https://x.test/ -d a=1 -d b=2");
        match &draft.body {
            BodyValue::Form { fields } => assert_eq!(fields.len(), 2),
            other => panic!("expected a form body, got {other:?}"),
        }
    }

    #[test]
    fn dash_g_moves_data_into_the_query_and_leaves_a_get() {
        let draft = parse("curl -G https://x.test/search -d q=hello -d limit=10");
        assert_eq!(draft.method, HttpMethod::Get);
        assert!(draft.body.is_none());
        let keys: Vec<_> = draft.query.iter().map(|q| q.key.as_str()).collect();
        assert_eq!(keys, vec!["q", "limit"]);
    }

    #[test]
    fn data_urlencode_percent_encodes_only_the_value() {
        let draft = parse("curl https://x.test/ --data-urlencode 'q=a b&c'");
        match &draft.body {
            BodyValue::Form { fields } => {
                assert_eq!(fields[0].key, "q");
                assert_eq!(fields[0].value, "a b&c");
            }
            other => panic!("expected a form body, got {other:?}"),
        }
    }

    #[test]
    fn multipart_forms_carry_text_and_file_parts() {
        let draft =
            parse("curl https://x.test/ -F name=Aryan -F 'avatar=@/tmp/a.png;type=image/png'");
        match &draft.body {
            BodyValue::Multipart { parts } => {
                assert!(matches!(&parts[0], FormPart::Text { value, .. } if value == "Aryan"));
                match &parts[1] {
                    FormPart::File {
                        path, content_type, ..
                    } => {
                        assert_eq!(path.to_string_lossy(), "/tmp/a.png");
                        assert_eq!(content_type.as_deref(), Some("image/png"));
                    }
                    other => panic!("expected a file part, got {other:?}"),
                }
            }
            other => panic!("expected a multipart body, got {other:?}"),
        }
    }

    #[test]
    fn transport_flags_map_onto_settings() {
        let draft = parse("curl -L -k -m 5 https://x.test/");
        assert!(draft.settings.follow_redirects);
        assert!(draft.settings.accept_invalid_certs);
        assert_eq!(draft.settings.timeout_ms, 5000);
    }

    #[test]
    fn insecure_imports_carry_a_warning() {
        let imported = parse_curl("curl -k https://x.test/").unwrap();
        assert!(imported.warnings.iter().any(|w| w.contains("-k")));
    }

    #[test]
    fn curl_managed_headers_are_dropped() {
        let draft = parse("curl https://x.test/ -H 'Content-Length: 12' -H 'Host: x.test' -d a=1");
        assert!(draft.headers.iter().all(|h| h.key != "Content-Length"));
        assert!(draft.headers.iter().all(|h| h.key != "Host"));
    }

    #[test]
    fn output_flags_are_dropped_without_complaint() {
        let imported = parse_curl("curl -s -v -o out.json https://x.test/").unwrap();
        assert_eq!(imported.value.url, "https://x.test/");
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn an_unknown_flag_is_reported_rather_than_silently_lost() {
        let imported = parse_curl("curl --made-up-flag https://x.test/").unwrap();
        assert!(imported.warnings.iter().any(|w| w.contains("made-up-flag")));
    }

    #[test]
    fn a_command_with_no_url_is_refused() {
        assert!(matches!(
            parse_curl("curl -X POST"),
            Err(ImportError::NoUrl)
        ));
    }

    #[test]
    fn a_flag_missing_its_value_is_reported() {
        assert!(matches!(
            parse_curl("curl https://x.test/ -H"),
            Err(ImportError::MissingValue { .. })
        ));
    }

    #[test]
    fn the_leading_curl_word_is_optional() {
        assert_eq!(parse("https://x.test/a").url, "https://x.test/a");
    }

    #[test]
    fn json_is_detected_without_a_content_type() {
        let draft = parse(r#"curl https://x.test/ -d '{"a":1}'"#);
        assert!(matches!(draft.body, BodyValue::Json { .. }));
    }

    #[test]
    fn the_json_flag_sets_method_and_content_type() {
        let draft = parse(r#"curl --json '{"a":1}' https://x.test/"#);
        assert_eq!(draft.method, HttpMethod::Post);
        assert!(matches!(draft.body, BodyValue::Json { .. }));
    }
}
