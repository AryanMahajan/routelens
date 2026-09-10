//! # rl-import
//!
//! Turns pasted or imported API definitions into [`rl_model`] types.
//!
//! - **cURL** — a real shell tokenizer first, flag parsing second. Commands copied from
//!   browser devtools are full of quoting that a naive whitespace split mangles.
//! - **OpenAPI** — 3.1, 3.0, and Swagger 2.0, with local `$ref` resolution.
//! - **Raw HTTP** — request line, headers, blank line, body.
//!
//! The guiding rule: the user should never have to decide by hand whether a pasted value
//! belongs in headers, query, auth, or body.
//!
//! Auth is always *structured* rather than left as a raw `Authorization` header, because that
//! is what lets a request round-trip — an environment switch can swap the token, and export
//! can re-render the header correctly.
//!
//! ## Nothing is silently dropped
//!
//! Every importer returns [`Imported`], which carries warnings alongside the result. An
//! unrecognised flag, a malformed header, a `$ref` that could not be resolved — each is
//! reported rather than discarded, so an import that lost something says so.
//!
//! ## Why the OpenAPI importer is built here rather than inside discovery
//!
//! Runtime enrich reuses it verbatim: `app.openapi()` output is just another OpenAPI
//! document. Building it once makes that path nearly free.

#![forbid(unsafe_code)]

pub mod curl;
pub mod error;
pub mod openapi;
pub mod raw_http;
pub mod shell;

pub use curl::parse_curl;
pub use error::{ImportError, Result};
pub use openapi::{parse_openapi, OpenApiImport};
pub use raw_http::parse_raw_http;
pub use shell::tokenize;

use rl_model::KeyValue;

/// An import result, with anything the importer could not fully honour.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported<T> {
    pub value: T,
    /// Empty when everything was understood.
    pub warnings: Vec<String>,
}

impl<T> Imported<T> {
    pub fn new(value: T) -> Self {
        Imported {
            value,
            warnings: Vec::new(),
        }
    }

    pub fn is_clean(&self) -> bool {
        self.warnings.is_empty()
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Imported<U> {
        Imported {
            value: f(self.value),
            warnings: self.warnings,
        }
    }
}

/// Split a URL into its base and structured query rows.
///
/// Both cURL and raw-HTTP import do this: a query string baked into a URL cannot have one
/// value swapped by an environment, but a row can.
pub(crate) fn split_url_query(raw: &str) -> Result<(String, Vec<KeyValue>)> {
    let parsed = url::Url::parse(raw).map_err(|source| ImportError::InvalidUrl {
        url: raw.to_string(),
        source,
    })?;

    let rows: Vec<KeyValue> = parsed
        .query_pairs()
        .map(|(k, v)| KeyValue::new(k.into_owned(), v.into_owned()))
        .collect();

    let mut base = parsed.clone();
    base.set_query(None);

    Ok((base.to_string(), rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_string_becomes_rows_and_leaves_the_base_clean() {
        let (base, rows) = split_url_query("https://x.test/users?page=2&q=a+b").unwrap();
        assert_eq!(base, "https://x.test/users");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].value, "a b", "values arrive decoded");
    }

    #[test]
    fn a_url_without_a_query_is_unchanged() {
        let (base, rows) = split_url_query("https://x.test/users").unwrap();
        assert_eq!(base, "https://x.test/users");
        assert!(rows.is_empty());
    }

    #[test]
    fn warnings_survive_mapping() {
        let imported = Imported {
            value: 1,
            warnings: vec!["careful".into()],
        };
        let mapped = imported.map(|n| n + 1);
        assert_eq!(mapped.value, 2);
        assert!(!mapped.is_clean());
    }
}
