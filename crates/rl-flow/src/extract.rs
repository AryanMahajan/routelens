//! Pulling a value out of a response.
//!
//! One function, [`value`], serves both extractions and assertions: it turns a
//! [`ValueSource`] into the text a later `{{variable}}` would carry, or `None` when there
//! is nothing there — a header the server did not send, a body path that does not resolve.
//!
//! "Nothing there" and "cannot look" are different answers. A body that is not JSON cannot
//! be walked at all, and saying so is more useful than a silent `None` that reads as "the
//! field was absent".

use rl_http::Exchange;
use rl_model::ValueSource;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExtractError {
    #[error("the response body is not JSON")]
    NotJson,

    #[error("malformed body path {path:?}: {reason}")]
    BadPath { path: String, reason: String },
}

/// The value a source names in this exchange, as text.
///
/// JSON scalars come back as their plain text — `"abc"` as `abc`, `42` as `42`, `true` as
/// `true` — because that is what a URL or header needs. Objects and arrays come back as
/// compact JSON, so a whole record can be passed along and re-sent as a body.
pub fn value(source: &ValueSource, exchange: &Exchange) -> Result<Option<String>, ExtractError> {
    let response = &exchange.response;
    Ok(match source {
        ValueSource::Status => Some(response.status.to_string()),
        ValueSource::Duration => Some(response.timing.total_ms.to_string()),
        ValueSource::Header { header } => response.header(header).map(str::to_string),
        ValueSource::BodyText => Some(response.body.as_text_lossy().into_owned()),
        ValueSource::Body { path } => {
            let document: Value =
                serde_json::from_slice(&response.body.bytes).map_err(|_| ExtractError::NotJson)?;
            walk(&document, path)?.map(render)
        }
    })
}

/// Follow a path like `user.id`, `items[0].name` or `$.data` through a JSON document.
pub fn walk<'a>(document: &'a Value, path: &str) -> Result<Option<&'a Value>, ExtractError> {
    let mut current = document;
    for segment in parse_path(path)? {
        let next = match segment {
            Segment::Key(key) => current.get(key),
            Segment::Index(index) => current.get(index),
        };
        match next {
            Some(value) => current = value,
            None => return Ok(None),
        }
    }
    Ok(Some(current))
}

/// The text form a `{{variable}}` carries.
pub fn render(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Segment<'a> {
    Key(&'a str),
    Index(usize),
}

/// `$.a.b[0].c` → `[Key(a), Key(b), Index(0), Key(c)]`. A bare `$` or empty path is the
/// whole document.
fn parse_path(path: &str) -> Result<Vec<Segment<'_>>, ExtractError> {
    let bad = |reason: &str| ExtractError::BadPath {
        path: path.to_string(),
        reason: reason.to_string(),
    };

    let mut rest = path.trim();
    if let Some(stripped) = rest.strip_prefix('$') {
        rest = stripped;
    }
    let mut segments = Vec::new();

    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            rest = after;
            if rest.is_empty() || rest.starts_with('.') || rest.starts_with('[') {
                return Err(bad("expected a key after `.`"));
            }
            continue;
        }
        if let Some(after) = rest.strip_prefix('[') {
            let close = after.find(']').ok_or_else(|| bad("unclosed `[`"))?;
            let inside = after[..close].trim();
            let segment = match inside.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                Some(quoted) => Segment::Key(quoted),
                None => Segment::Index(
                    inside
                        .parse()
                        .map_err(|_| bad(&format!("`[{inside}]` is not an index")))?,
                ),
            };
            segments.push(segment);
            rest = &after[close + 1..];
            continue;
        }
        let end = rest.find(['.', '[']).unwrap_or(rest.len());
        segments.push(Segment::Key(&rest[..end]));
        rest = &rest[end..];
    }

    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_http::{Body, Response, SentRequest, Timing};

    fn exchange(status: u16, body: &str, headers: &[(&str, &str)]) -> Exchange {
        Exchange {
            request: SentRequest {
                method: "GET".into(),
                url: "http://x.test/".into(),
                headers: vec![],
                body_size: 0,
                body_preview: None,
            },
            response: Response {
                status,
                status_text: String::new(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body: Body {
                    bytes: body.as_bytes().to_vec(),
                    truncated: false,
                    reported_length: None,
                    content_type: Some("application/json".into()),
                    content_encoding: None,
                },
                timing: Timing {
                    ttfb_ms: 3,
                    total_ms: 7,
                },
                redirects: vec![],
                insecure: false,
            },
        }
    }

    fn body(path: &str) -> ValueSource {
        ValueSource::Body {
            path: path.to_string(),
        }
    }

    #[test]
    fn scalars_come_back_as_plain_text_not_json_literals() {
        let e = exchange(
            200,
            r#"{"user":{"id":42,"name":"Aryan","admin":true,"nick":null}}"#,
            &[],
        );
        assert_eq!(value(&body("user.id"), &e).unwrap().as_deref(), Some("42"));
        assert_eq!(
            value(&body("user.name"), &e).unwrap().as_deref(),
            Some("Aryan")
        );
        assert_eq!(
            value(&body("user.admin"), &e).unwrap().as_deref(),
            Some("true")
        );
        assert_eq!(
            value(&body("user.nick"), &e).unwrap().as_deref(),
            Some("null")
        );
    }

    #[test]
    fn objects_and_arrays_come_back_as_compact_json() {
        let e = exchange(200, r#"{"items":[{"id":1},{"id":2}]}"#, &[]);
        assert_eq!(
            value(&body("items"), &e).unwrap().as_deref(),
            Some(r#"[{"id":1},{"id":2}]"#)
        );
        assert_eq!(
            value(&body("items[1].id"), &e).unwrap().as_deref(),
            Some("2")
        );
        assert_eq!(
            value(&body("$"), &e).unwrap().as_deref(),
            Some(r#"{"items":[{"id":1},{"id":2}]}"#)
        );
    }

    #[test]
    fn every_spelling_of_the_root_and_index_forms_is_accepted() {
        let doc: Value = serde_json::json!({"a": [ {"b": "x"} ], "k y": 1});
        for path in ["a[0].b", "$.a[0].b", "$a[0].b", ".a[0].b"] {
            assert_eq!(
                walk(&doc, path).map(|v| v.map(render)),
                Ok(Some("x".to_string())),
                "{path}"
            );
        }
        assert_eq!(
            walk(&doc, r#"["k y"]"#).unwrap().map(render),
            Some("1".into())
        );
        assert_eq!(walk(&doc, "").unwrap(), Some(&doc));
    }

    #[test]
    fn a_missing_field_is_none_not_an_error() {
        let e = exchange(200, r#"{"user":{"id":42}}"#, &[]);
        assert_eq!(value(&body("user.email"), &e).unwrap(), None);
        assert_eq!(value(&body("user.id.deeper"), &e).unwrap(), None);
        assert_eq!(value(&body("items[3]"), &e).unwrap(), None);
    }

    #[test]
    fn a_body_that_is_not_json_says_so_rather_than_pretending_the_field_is_absent() {
        let e = exchange(200, "<html>", &[]);
        assert_eq!(value(&body("user.id"), &e), Err(ExtractError::NotJson));
    }

    #[test]
    fn a_malformed_path_is_refused() {
        let e = exchange(200, "{}", &[]);
        for bad in ["a[", "a[x]", "a..b", "a."] {
            assert!(
                matches!(value(&body(bad), &e), Err(ExtractError::BadPath { .. })),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn status_headers_duration_and_raw_text_are_all_reachable() {
        let e = exchange(201, "created", &[("Location", "/users/7")]);
        assert_eq!(
            value(&ValueSource::Status, &e).unwrap().as_deref(),
            Some("201")
        );
        assert_eq!(
            value(
                &ValueSource::Header {
                    header: "location".into()
                },
                &e
            )
            .unwrap()
            .as_deref(),
            Some("/users/7")
        );
        assert_eq!(
            value(
                &ValueSource::Header {
                    header: "ETag".into()
                },
                &e
            )
            .unwrap(),
            None
        );
        assert_eq!(
            value(&ValueSource::Duration, &e).unwrap().as_deref(),
            Some("7")
        );
        assert_eq!(
            value(&ValueSource::BodyText, &e).unwrap().as_deref(),
            Some("created")
        );
    }
}
