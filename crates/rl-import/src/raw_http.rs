//! Raw HTTP import.
//!
//! For pasting out of a log, a proxy, or a `.http` file.

use crate::error::{ImportError, Result};
use crate::Imported;
use rl_model::{AuthConfig, HttpMethod, KeyValue, RequestDraft};

/// Parse a raw HTTP request.
///
/// ```text
/// POST /api/v1/users HTTP/1.1
/// Host: api.example.com
/// Content-Type: application/json
///
/// {"name": "Aryan"}
/// ```
///
/// The URL is reconstructed from `Host` plus the request target, and the same
/// auth-structuring rules as cURL import apply.
pub fn parse_raw_http(input: &str) -> Result<Imported<RequestDraft>> {
    let normalized = input.replace("\r\n", "\n");
    let trimmed = normalized.trim_start_matches('\n');
    if trimmed.trim().is_empty() {
        return Err(ImportError::Empty);
    }

    // The head ends at the first blank line; everything after is body.
    let (head, body) = match trimmed.split_once("\n\n") {
        Some((head, body)) => (head, Some(body)),
        None => (trimmed.trim_end(), None),
    };

    let mut lines = head.lines();
    let request_line = lines.next().ok_or(ImportError::Empty)?;

    let mut parts = request_line.split_whitespace();
    let method_text = parts.next().ok_or_else(|| ImportError::BadRequestLine {
        line: request_line.to_string(),
    })?;
    let target = parts.next().ok_or_else(|| ImportError::BadRequestLine {
        line: request_line.to_string(),
    })?;

    let method: HttpMethod = method_text.parse().unwrap_or(HttpMethod::Get);

    let mut warnings = Vec::new();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut host: Option<String> = None;
    let mut scheme = "http";

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            warnings.push(format!("ignored malformed header line `{line}`"));
            continue;
        };
        let name = name.trim().to_string();
        let value = value.trim().to_string();

        if name.eq_ignore_ascii_case("host") {
            host = Some(value.clone());
            continue;
        }
        // A proxy transcript often records the scheme it used.
        if name.eq_ignore_ascii_case("x-forwarded-proto") && value == "https" {
            scheme = "https";
            continue;
        }
        headers.push((name, value));
    }

    let url = if target.contains("://") {
        target.to_string()
    } else {
        match &host {
            Some(host) => {
                // A host on 443 is being spoken to over TLS whatever the transcript says.
                let scheme = if host.ends_with(":443") { "https" } else { scheme };
                format!("{scheme}://{host}{target}")
            }
            None => {
                warnings.push(
                    "no Host header and a relative target, so the URL is incomplete".into(),
                );
                target.to_string()
            }
        }
    };

    let mut draft = RequestDraft::new(method, "");

    match crate::split_url_query(&url) {
        Ok((base, query)) => {
            draft.url = base;
            draft.query = query;
        }
        Err(_) => draft.url = url,
    }

    let mut content_type = None;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("authorization") {
            if let Some(auth) = crate::curl::parse_authorization_header(&value) {
                draft.auth = auth;
                continue;
            }
        }
        if name.eq_ignore_ascii_case("cookie") {
            for (key, val) in crate::curl::parse_cookies(&value) {
                draft.cookies.push(KeyValue::new(key, val));
            }
            continue;
        }
        if name.eq_ignore_ascii_case("content-type") {
            content_type = Some(value.clone());
        }
        // Recomputed at send time; a transcript's value would be stale.
        if name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        draft.headers.push(KeyValue::new(name, value));
    }

    if let Some(body) = body {
        let body = body.trim_end_matches('\n');
        if !body.is_empty() {
            draft.body = crate::curl::body_from_text(body, content_type.as_deref());
            draft
                .headers
                .retain(|h| !h.key.eq_ignore_ascii_case("content-type"));
        }
    }

    if matches!(draft.auth, AuthConfig::None) && draft.body.is_none() && draft.headers.is_empty() {
        warnings.push("no headers or body were found; check the paste is complete".into());
    }

    Ok(Imported {
        value: draft,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_model::BodyValue;

    fn parse(input: &str) -> RequestDraft {
        parse_raw_http(input).unwrap().value
    }

    #[test]
    fn the_documented_example_parses() {
        let draft = parse(
            "POST /api/v1/users HTTP/1.1\n\
             Host: api.example.com\n\
             Content-Type: application/json\n\
             Authorization: Bearer token\n\
             \n\
             {\"name\": \"Aryan\"}",
        );

        assert_eq!(draft.method, HttpMethod::Post);
        assert_eq!(draft.url, "http://api.example.com/api/v1/users");
        assert_eq!(
            draft.auth,
            AuthConfig::Bearer {
                token: "token".into()
            }
        );
        match &draft.body {
            BodyValue::Json { content } => assert_eq!(content, "{\"name\": \"Aryan\"}"),
            other => panic!("expected a json body, got {other:?}"),
        }
    }

    #[test]
    fn windows_line_endings_are_handled() {
        let draft = parse("GET /health HTTP/1.1\r\nHost: x.test\r\n\r\n");
        assert_eq!(draft.url, "http://x.test/health");
    }

    #[test]
    fn a_query_string_is_split_into_rows() {
        let draft = parse("GET /users?page=2&limit=10 HTTP/1.1\nHost: x.test");
        assert_eq!(draft.url, "http://x.test/users");
        let keys: Vec<_> = draft.query.iter().map(|q| q.key.as_str()).collect();
        assert_eq!(keys, vec!["page", "limit"]);
    }

    #[test]
    fn an_absolute_target_is_used_as_is() {
        let draft = parse("GET https://api.example.com/x HTTP/1.1\nHost: ignored.test");
        assert_eq!(draft.url, "https://api.example.com/x");
    }

    #[test]
    fn a_host_on_443_implies_https() {
        let draft = parse("GET /x HTTP/1.1\nHost: api.example.com:443");
        assert!(draft.url.starts_with("https://"));
    }

    #[test]
    fn a_request_with_no_body_has_none() {
        let draft = parse("GET /health HTTP/1.1\nHost: x.test");
        assert!(draft.body.is_none());
    }

    #[test]
    fn content_length_is_dropped_because_it_is_recomputed() {
        let draft = parse("POST /x HTTP/1.1\nHost: x.test\nContent-Length: 9\n\n{\"a\": 1}");
        assert!(draft.headers.iter().all(|h| h.key != "Content-Length"));
    }

    #[test]
    fn cookies_become_rows() {
        let draft = parse("GET /x HTTP/1.1\nHost: x.test\nCookie: a=1; b=2");
        assert_eq!(draft.cookies.len(), 2);
    }

    #[test]
    fn a_missing_host_is_flagged_rather_than_guessed() {
        let imported = parse_raw_http("GET /x HTTP/1.1\nAccept: */*").unwrap();
        assert!(imported.warnings.iter().any(|w| w.contains("Host")));
    }

    #[test]
    fn an_empty_paste_is_refused() {
        assert!(matches!(parse_raw_http("   \n  "), Err(ImportError::Empty)));
    }

    #[test]
    fn a_malformed_request_line_is_refused() {
        assert!(matches!(
            parse_raw_http("GARBAGE"),
            Err(ImportError::BadRequestLine { .. })
        ));
    }

    #[test]
    fn a_body_containing_a_blank_line_is_kept_whole() {
        let draft = parse("POST /x HTTP/1.1\nHost: x.test\nContent-Type: text/plain\n\nline one\n\nline three");
        match &draft.body {
            BodyValue::Text { content, .. } => {
                assert!(content.contains("line one"));
                assert!(content.contains("line three"));
            }
            other => panic!("expected a text body, got {other:?}"),
        }
    }
}
