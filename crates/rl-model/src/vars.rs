//! Variable resolution.
//!
//! `{{name}}` interpolation, with one implementation shared by the UI preview and the HTTP
//! engine. That sharing is the point: the request shown in the preview is byte-for-byte the
//! request that gets sent, because the same code produced both.
//!
//! Precedence, highest first:
//!
//! 1. request-local
//! 2. environment
//! 3. workspace globals
//! 4. secrets
//!
//! Secrets sit last so an ordinary variable always wins a name collision. Referencing one
//! explicitly with `{{secret:name}}` is the documented form — the prefix makes credential
//! use visible when reading a committed file.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Guards against a variable chain that never terminates.
pub const MAX_DEPTH: usize = 10;

/// The mask substituted for a secret value wherever one would otherwise be written down.
pub const REDACTION: &str = "••••••";

const SECRET_PREFIX: &str = "secret:";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("variable `{name}` is not defined")]
    Undefined { name: String },

    #[error("variable `{name}` refers to itself")]
    Cycle { name: String },

    #[error("variable nesting exceeded {} levels", MAX_DEPTH)]
    DepthExceeded,

    #[error("empty variable reference `{{{{}}}}`")]
    EmptyName,
}

/// The variables available to a request, in precedence order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableContext {
    #[serde(default)]
    pub request_local: BTreeMap<String, String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub globals: BTreeMap<String, String>,
    /// Never serialized as part of a workspace. Values live in the private tier.
    #[serde(skip)]
    pub secrets: BTreeMap<String, String>,
}

/// A resolved string, plus the secrets that went into it.
///
/// `secrets_used` is what makes redaction possible: history and exports need to know which
/// values must not be written down, and only the resolver knows which were substituted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub value: String,
    pub secrets_used: BTreeSet<String>,
}

impl Resolved {
    pub fn uses_secrets(&self) -> bool {
        !self.secrets_used.is_empty()
    }
}

impl VariableContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_environment(mut self, vars: BTreeMap<String, String>) -> Self {
        self.environment = vars;
        self
    }

    pub fn with_secrets(mut self, secrets: BTreeMap<String, String>) -> Self {
        self.secrets = secrets;
        self
    }

    pub fn set_local(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.request_local.insert(name.into(), value.into());
    }

    /// Look a name up, reporting whether the hit was a secret.
    fn lookup(&self, name: &str) -> Option<(&str, bool)> {
        if let Some(secret_name) = name.strip_prefix(SECRET_PREFIX) {
            let key = secret_name.trim();
            return self.secrets.get(key).map(|v| (v.as_str(), true));
        }

        self.request_local
            .get(name)
            .or_else(|| self.environment.get(name))
            .or_else(|| self.globals.get(name))
            .map(|v| (v.as_str(), false))
            .or_else(|| self.secrets.get(name).map(|v| (v.as_str(), true)))
    }

    /// Whether a name resolves at all.
    pub fn is_defined(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    /// Substitute every `{{name}}` in `input`.
    ///
    /// A missing variable is an error rather than an empty string. Silently producing
    /// `https:///api/users` fails later in a way the developer will blame on their own code.
    pub fn resolve(&self, input: &str) -> Result<Resolved, ResolveError> {
        let mut secrets_used = BTreeSet::new();
        let mut stack = Vec::new();
        let value = self.expand(input, 0, &mut stack, &mut secrets_used)?;
        Ok(Resolved {
            value,
            secrets_used,
        })
    }

    fn expand(
        &self,
        input: &str,
        depth: usize,
        stack: &mut Vec<String>,
        secrets_used: &mut BTreeSet<String>,
    ) -> Result<String, ResolveError> {
        if depth > MAX_DEPTH {
            return Err(ResolveError::DepthExceeded);
        }

        let mut out = String::with_capacity(input.len());
        let mut rest = input;

        while let Some(open) = rest.find("{{") {
            out.push_str(&rest[..open]);
            let after_open = &rest[open + 2..];

            let Some(close) = after_open.find("}}") else {
                // No closing delimiter: the rest is literal text, not a broken reference.
                out.push_str(&rest[open..]);
                return Ok(out);
            };

            let name = after_open[..close].trim();
            if name.is_empty() {
                return Err(ResolveError::EmptyName);
            }

            let (raw_value, is_secret) = self
                .lookup(name)
                .ok_or_else(|| ResolveError::Undefined {
                    name: name.to_string(),
                })
                .map(|(v, s)| (v.to_string(), s))?;

            if is_secret {
                let key = name.strip_prefix(SECRET_PREFIX).unwrap_or(name).trim();
                secrets_used.insert(key.to_string());
            }

            // A variable may itself contain references. Track the chain to catch cycles.
            if stack.iter().any(|n| n == name) {
                return Err(ResolveError::Cycle {
                    name: name.to_string(),
                });
            }
            stack.push(name.to_string());
            let expanded = self.expand(&raw_value, depth + 1, stack, secrets_used)?;
            stack.pop();

            out.push_str(&expanded);
            rest = &after_open[close + 2..];
        }

        out.push_str(rest);
        Ok(out)
    }

    /// Every `{{name}}` referenced by `input`, in order of first appearance.
    ///
    /// Used by the UI to show which variables a request depends on, and to report the ones
    /// that are not defined *before* the request is sent.
    pub fn references(input: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = input;

        while let Some(open) = rest.find("{{") {
            let after_open = &rest[open + 2..];
            let Some(close) = after_open.find("}}") else {
                break;
            };
            let name = after_open[..close].trim();
            if !name.is_empty() && !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
            rest = &after_open[close + 2..];
        }

        names
    }

    /// The referenced names that do not resolve.
    pub fn undefined_references(&self, input: &str) -> Vec<String> {
        Self::references(input)
            .into_iter()
            .filter(|n| !self.is_defined(n))
            .collect()
    }

    /// Replace the given secrets' values with [`REDACTION`].
    ///
    /// Applied wherever resolved text leaves the private tier — history rows, exports, logs,
    /// error messages. Empty secret values are skipped, since replacing an empty string would
    /// mask the entire text.
    pub fn redact(&self, text: &str, secrets_used: &BTreeSet<String>) -> String {
        let mut out = text.to_string();
        for name in secrets_used {
            if let Some(value) = self.secrets.get(name) {
                if !value.is_empty() {
                    out = out.replace(value.as_str(), REDACTION);
                }
            }
        }
        out
    }

    /// Redact every known secret, regardless of what was recorded as used.
    ///
    /// The belt-and-braces form, for text that did not come from [`Self::resolve`] — a
    /// response body that happens to echo a token back, for instance.
    pub fn redact_all(&self, text: &str) -> String {
        let mut out = text.to_string();
        for value in self.secrets.values() {
            if !value.is_empty() {
                out = out.replace(value.as_str(), REDACTION);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> VariableContext {
        let mut c = VariableContext::new();
        c.globals.insert("scheme".into(), "https".into());
        c.environment
            .insert("base_url".into(), "http://localhost:8000".into());
        c.environment.insert("api_version".into(), "v1".into());
        c.secrets.insert("api_token".into(), "s3cr3t-value".into());
        c
    }

    #[test]
    fn substitutes_a_simple_reference() {
        let r = ctx().resolve("{{base_url}}/health").unwrap();
        assert_eq!(r.value, "http://localhost:8000/health");
        assert!(!r.uses_secrets());
    }

    #[test]
    fn substitutes_several_in_one_string() {
        let r = ctx().resolve("{{base_url}}/api/{{api_version}}/users").unwrap();
        assert_eq!(r.value, "http://localhost:8000/api/v1/users");
    }

    #[test]
    fn tolerates_whitespace_inside_the_braces() {
        let r = ctx().resolve("{{ base_url }}/x").unwrap();
        assert_eq!(r.value, "http://localhost:8000/x");
    }

    #[test]
    fn precedence_runs_local_then_environment_then_globals() {
        let mut c = ctx();
        c.globals.insert("who".into(), "global".into());
        assert_eq!(c.resolve("{{who}}").unwrap().value, "global");

        c.environment.insert("who".into(), "environment".into());
        assert_eq!(c.resolve("{{who}}").unwrap().value, "environment");

        c.set_local("who", "local");
        assert_eq!(c.resolve("{{who}}").unwrap().value, "local");
    }

    #[test]
    fn an_ordinary_variable_wins_over_a_same_named_secret() {
        let mut c = ctx();
        c.secrets.insert("shared".into(), "from-secrets".into());
        c.environment.insert("shared".into(), "from-env".into());

        let r = c.resolve("{{shared}}").unwrap();
        assert_eq!(r.value, "from-env");
        assert!(!r.uses_secrets(), "must not be recorded as a secret use");
    }

    #[test]
    fn secrets_resolve_and_are_recorded() {
        let r = ctx().resolve("Bearer {{secret:api_token}}").unwrap();
        assert_eq!(r.value, "Bearer s3cr3t-value");
        assert!(r.secrets_used.contains("api_token"));
    }

    #[test]
    fn secrets_are_reachable_as_a_last_resort_and_still_recorded() {
        let r = ctx().resolve("{{api_token}}").unwrap();
        assert_eq!(r.value, "s3cr3t-value");
        assert!(r.secrets_used.contains("api_token"));
    }

    #[test]
    fn a_missing_variable_is_an_error_not_an_empty_string() {
        let err = ctx().resolve("{{base_url}}/{{nope}}").unwrap_err();
        assert_eq!(
            err,
            ResolveError::Undefined {
                name: "nope".into()
            }
        );
    }

    #[test]
    fn nested_references_expand() {
        let mut c = ctx();
        c.environment
            .insert("root".into(), "{{scheme}}://example.com".into());
        assert_eq!(
            c.resolve("{{root}}/users").unwrap().value,
            "https://example.com/users"
        );
    }

    #[test]
    fn a_self_referential_variable_is_caught() {
        let mut c = ctx();
        c.environment.insert("loop".into(), "{{loop}}".into());
        assert_eq!(
            c.resolve("{{loop}}").unwrap_err(),
            ResolveError::Cycle { name: "loop".into() }
        );
    }

    #[test]
    fn a_mutual_cycle_is_caught() {
        let mut c = ctx();
        c.environment.insert("a".into(), "{{b}}".into());
        c.environment.insert("b".into(), "{{a}}".into());
        assert!(matches!(
            c.resolve("{{a}}").unwrap_err(),
            ResolveError::Cycle { .. }
        ));
    }

    #[test]
    fn an_unclosed_brace_is_literal_text() {
        let r = ctx().resolve("{{base_url}}/a{{b").unwrap();
        assert_eq!(r.value, "http://localhost:8000/a{{b");
    }

    #[test]
    fn an_empty_reference_is_rejected() {
        assert_eq!(ctx().resolve("{{}}").unwrap_err(), ResolveError::EmptyName);
    }

    #[test]
    fn text_without_references_passes_through_unchanged() {
        let body = r#"{"name": "Aryan", "nested": {"a": 1}}"#;
        assert_eq!(ctx().resolve(body).unwrap().value, body);
    }

    #[test]
    fn handles_multibyte_text() {
        let mut c = ctx();
        c.environment.insert("greeting".into(), "héllo→".into());
        let r = c.resolve("→ {{greeting}} ←").unwrap();
        assert_eq!(r.value, "→ héllo→ ←");
    }

    #[test]
    fn references_are_listed_in_order_without_duplicates() {
        let found = VariableContext::references("{{a}}/{{b}}/{{a}}/{{ c }}");
        assert_eq!(found, vec!["a", "b", "c"]);
    }

    #[test]
    fn undefined_references_are_reportable_before_sending() {
        let c = ctx();
        let missing = c.undefined_references("{{base_url}}/{{nope}}/{{also_missing}}");
        assert_eq!(missing, vec!["nope", "also_missing"]);
    }

    #[test]
    fn redaction_masks_the_value_that_was_used() {
        let c = ctx();
        let r = c.resolve("Authorization: Bearer {{secret:api_token}}").unwrap();
        let safe = c.redact(&r.value, &r.secrets_used);
        assert_eq!(safe, format!("Authorization: Bearer {REDACTION}"));
        assert!(!safe.contains("s3cr3t-value"));
    }

    #[test]
    fn redact_all_catches_a_secret_echoed_back_by_a_server() {
        let c = ctx();
        let response = r#"{"token": "s3cr3t-value"}"#;
        assert!(!c.redact_all(response).contains("s3cr3t-value"));
    }

    #[test]
    fn an_empty_secret_does_not_mask_everything() {
        let mut c = ctx();
        c.secrets.insert("blank".into(), String::new());
        assert_eq!(c.redact_all("untouched"), "untouched");
    }

    #[test]
    fn secrets_are_never_serialized_with_the_context() {
        let json = serde_json::to_string(&ctx()).unwrap();
        assert!(!json.contains("s3cr3t-value"));
        assert!(!json.contains("api_token"));
    }
}
