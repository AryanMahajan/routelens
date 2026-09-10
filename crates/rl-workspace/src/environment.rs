//! Environments — named variable sets, committed.
//!
//! Switching environments repoints every request in the workspace without editing any of
//! them.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const CURRENT_VERSION: u32 = 1;

/// A named set of variables.
///
/// `variables` are visible and committed. `secrets` lists **names only** — values live in
/// the private tier and are referenced from requests as `{{secret:name}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    #[serde(default = "default_version")]
    pub version: u32,
    pub name: String,
    /// `BTreeMap` rather than a hash map: sorted keys mean a diff reflects a real edit, not
    /// serializer churn. These files are meant to be reviewed in a pull request.
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
    /// The secret names this environment expects. Never values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
}

fn default_version() -> u32 {
    CURRENT_VERSION
}

impl Environment {
    pub fn new(name: impl Into<String>) -> Self {
        Environment {
            version: CURRENT_VERSION,
            name: name.into(),
            variables: BTreeMap::new(),
            secrets: Vec::new(),
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.variables.insert(key.into(), value.into());
        self
    }

    /// Declare that this environment expects a secret of this name.
    pub fn expect_secret(&mut self, name: impl Into<String>) -> &mut Self {
        let name = name.into();
        if !self.secrets.contains(&name) {
            self.secrets.push(name);
        }
        self
    }

    /// The expected secrets that the store does not actually hold.
    ///
    /// This is what turns a confusing 401 into a clear "you have not set `api_token` yet".
    pub fn missing_secrets(&self, available: &BTreeMap<String, String>) -> Vec<String> {
        self.secrets
            .iter()
            .filter(|name| !available.contains_key(*name))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> Environment {
        let mut env = Environment::new("local");
        env.set("base_url", "http://localhost:8000")
            .set("api_version", "v1")
            .expect_secret("api_token");
        env
    }

    #[test]
    fn round_trips_through_yaml() {
        let env = local();
        let text = yaml_serde::to_string(&env).unwrap();
        let back: Environment = yaml_serde::from_str(&text).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn a_committed_environment_names_secrets_but_never_holds_values() {
        let text = yaml_serde::to_string(&local()).unwrap();
        assert!(text.contains("api_token"));
        assert!(!text.contains("s3cr3t"));
    }

    #[test]
    fn variables_serialize_in_sorted_order_so_diffs_stay_meaningful() {
        let mut env = Environment::new("local");
        env.set("zulu", "1").set("alpha", "2").set("mike", "3");

        let text = yaml_serde::to_string(&env).unwrap();
        let alpha = text.find("alpha").unwrap();
        let mike = text.find("mike").unwrap();
        let zulu = text.find("zulu").unwrap();
        assert!(alpha < mike && mike < zulu);
    }

    #[test]
    fn expecting_the_same_secret_twice_does_not_duplicate_it() {
        let mut env = Environment::new("local");
        env.expect_secret("api_token").expect_secret("api_token");
        assert_eq!(env.secrets, vec!["api_token"]);
    }

    #[test]
    fn unset_secrets_are_reportable_before_a_request_fails() {
        let env = local();
        assert_eq!(env.missing_secrets(&BTreeMap::new()), vec!["api_token"]);

        let mut available = BTreeMap::new();
        available.insert("api_token".to_string(), "s3cr3t".to_string());
        assert!(env.missing_secrets(&available).is_empty());
    }
}
