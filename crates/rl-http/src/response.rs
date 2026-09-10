//! What came back.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// How much of a response body is kept in memory for display.
///
/// Beyond this the body is truncated and the UI offers to save the whole thing to a file
/// instead. A 500 MB download must not take the application with it.
pub const MAX_BODY_PREVIEW: usize = 8 * 1024 * 1024;

/// Timing for one exchange.
///
/// P1 ships time-to-first-byte and total. The DNS / TCP / TLS breakdown needs a custom
/// connector and is deliberately deferred rather than allowed to block the runner —
/// see `docs/architecture.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Timing {
    /// Request sent to response headers received.
    pub ttfb_ms: u64,
    /// Request sent to response body fully read.
    pub total_ms: u64,
}

/// One step in a redirect chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hop {
    pub status: u16,
    pub from: String,
    pub to: String,
    /// The `Authorization` header was dropped because the redirect crossed origins.
    #[serde(default)]
    pub credentials_stripped: bool,
}

/// A response body, as received.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Body {
    #[serde(with = "serde_bytes_vec")]
    pub bytes: Vec<u8>,
    /// The body was longer than [`MAX_BODY_PREVIEW`] and only the first part is here.
    #[serde(default)]
    pub truncated: bool,
    /// Total length as reported by the server, when it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// The `Content-Encoding` the server applied.
    ///
    /// Recorded rather than hidden: the engine transparently decodes gzip, brotli and
    /// deflate so the body is readable, and this field is what makes that visible instead of
    /// leaving the developer wondering why the bytes do not match `curl --raw`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_encoding: Option<String>,
}

/// `Vec<u8>` as a JSON array is wasteful but honest, and history rows are not hot.
mod serde_bytes_vec {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&String::from_utf8_lossy(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        Ok(text.into_bytes())
    }
}

impl Body {
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The body as text, if it is valid UTF-8.
    pub fn as_text(&self) -> Option<Cow<'_, str>> {
        std::str::from_utf8(&self.bytes).ok().map(Cow::Borrowed)
    }

    /// The body as text, replacing invalid sequences. For display only.
    pub fn as_text_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.bytes)
    }

    /// Pretty-printed, when the body parses as JSON.
    pub fn pretty_json(&self) -> Option<String> {
        let value: serde_json::Value = serde_json::from_slice(&self.bytes).ok()?;
        serde_json::to_string_pretty(&value).ok()
    }

    /// Whether the content type suggests text the UI can display inline.
    pub fn looks_textual(&self) -> bool {
        let Some(ct) = &self.content_type else {
            return self.as_text().is_some();
        };
        let ct = ct.to_ascii_lowercase();
        ct.starts_with("text/")
            || ct.contains("json")
            || ct.contains("xml")
            || ct.contains("javascript")
            || ct.contains("x-www-form-urlencoded")
    }
}

/// What actually went out on the wire.
///
/// Recorded separately from the draft because they differ: variables are resolved, auth has
/// become a header, and disabled rows are gone. When a request behaves unexpectedly this is
/// the thing worth looking at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body_size: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_preview: Option<String>,
}

/// A response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub status: u16,
    pub status_text: String,
    /// In the order the server sent them, duplicates included.
    pub headers: Vec<(String, String)>,
    pub body: Body,
    pub timing: Timing,
    /// Empty unless redirects were followed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirects: Vec<Hop>,
    /// Certificate verification was disabled for this exchange.
    #[serde(default)]
    pub insecure: bool,
}

impl Response {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn is_redirect(&self) -> bool {
        (300..400).contains(&self.status)
    }

    pub fn is_error(&self) -> bool {
        self.status >= 400
    }

    /// First value for a header name, case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A request and its response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exchange {
    pub request: SentRequest,
    pub response: Response,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(bytes: &[u8], content_type: Option<&str>) -> Body {
        Body {
            bytes: bytes.to_vec(),
            truncated: false,
            reported_length: None,
            content_type: content_type.map(str::to_string),
            content_encoding: None,
        }
    }

    #[test]
    fn json_bodies_pretty_print() {
        let b = body(br#"{"a":1,"b":[2,3]}"#, Some("application/json"));
        let pretty = b.pretty_json().unwrap();
        assert!(pretty.contains("\n"));
        assert!(pretty.contains("\"a\": 1"));
    }

    #[test]
    fn non_json_bodies_do_not_pretty_print() {
        assert!(body(b"<html></html>", Some("text/html"))
            .pretty_json()
            .is_none());
    }

    #[test]
    fn invalid_utf8_is_not_text_but_is_still_displayable() {
        let b = body(&[0xff, 0xfe, 0x00], Some("application/octet-stream"));
        assert!(b.as_text().is_none());
        assert!(!b.as_text_lossy().is_empty());
    }

    #[test]
    fn textual_content_types_are_recognised() {
        assert!(body(b"{}", Some("application/json")).looks_textual());
        assert!(body(b"x", Some("text/plain; charset=utf-8")).looks_textual());
        assert!(body(b"<a/>", Some("application/xml")).looks_textual());
        assert!(!body(&[0xff], Some("image/png")).looks_textual());
    }

    #[test]
    fn a_body_with_no_content_type_falls_back_to_sniffing() {
        assert!(body(b"plain text", None).looks_textual());
        assert!(!body(&[0xff, 0xfe], None).looks_textual());
    }

    #[test]
    fn header_lookup_ignores_case() {
        let r = Response {
            status: 200,
            status_text: "OK".into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Body::default(),
            timing: Timing::default(),
            redirects: Vec::new(),
            insecure: false,
        };
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_eq!(r.header("CONTENT-TYPE"), Some("application/json"));
        assert_eq!(r.header("missing"), None);
    }

    #[test]
    fn status_classes() {
        let mk = |status| Response {
            status,
            status_text: String::new(),
            headers: vec![],
            body: Body::default(),
            timing: Timing::default(),
            redirects: vec![],
            insecure: false,
        };
        assert!(mk(204).is_success());
        assert!(mk(301).is_redirect());
        assert!(mk(404).is_error());
        assert!(mk(500).is_error());
    }
}
