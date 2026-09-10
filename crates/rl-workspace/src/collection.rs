//! Collections — saved requests, committed alongside the project.
//!
//! The intended payoff: someone clones the repository, opens RouteLens, and the project's
//! requests are already there.

use rl_model::RequestDraft;
use serde::{Deserialize, Serialize};

pub const CURRENT_VERSION: u32 = 1;

/// A named group of saved requests, stored as one file.
///
/// One collection per file means two people adding requests to different collections never
/// conflict, and request order is the order in the file — no sort key to churn in diffs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    #[serde(default = "default_version")]
    pub version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub requests: Vec<RequestDraft>,
}

fn default_version() -> u32 {
    CURRENT_VERSION
}

impl Collection {
    pub fn new(name: impl Into<String>) -> Self {
        Collection {
            version: CURRENT_VERSION,
            name: name.into(),
            description: None,
            requests: Vec::new(),
        }
    }

    pub fn push(&mut self, request: RequestDraft) {
        self.requests.push(request);
    }

    /// Find a request by name. Names are how a person refers to a saved request, so this is
    /// the lookup the UI actually needs.
    pub fn find(&self, name: &str) -> Option<&RequestDraft> {
        self.requests
            .iter()
            .find(|r| r.name.as_deref() == Some(name))
    }

    pub fn remove(&mut self, name: &str) -> Option<RequestDraft> {
        let index = self
            .requests
            .iter()
            .position(|r| r.name.as_deref() == Some(name))?;
        Some(self.requests.remove(index))
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    pub fn len(&self) -> usize {
        self.requests.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_model::{AuthConfig, HttpMethod};

    fn request(name: &str) -> RequestDraft {
        let mut r = RequestDraft::new(HttpMethod::Get, "{{base_url}}/users");
        r.name = Some(name.to_string());
        r
    }

    #[test]
    fn requests_keep_their_insertion_order() {
        let mut c = Collection::new("Users");
        c.push(request("List users"));
        c.push(request("Create user"));

        let names: Vec<_> = c
            .requests
            .iter()
            .filter_map(|r| r.name.as_deref())
            .collect();
        assert_eq!(names, vec!["List users", "Create user"]);
    }

    #[test]
    fn requests_are_findable_and_removable_by_name() {
        let mut c = Collection::new("Users");
        c.push(request("List users"));

        assert!(c.find("List users").is_some());
        assert!(c.remove("List users").is_some());
        assert!(c.is_empty());
    }

    #[test]
    fn round_trips_through_yaml() {
        let mut c = Collection::new("Users");
        c.push(request("List users"));

        let text = yaml_serde::to_string(&c).unwrap();
        let back: Collection = yaml_serde::from_str(&text).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn a_saved_request_keeps_the_secret_reference_not_the_value() {
        let mut r = request("Authenticated");
        r.auth = AuthConfig::Bearer {
            token: "{{secret:api_token}}".into(),
        };
        let mut c = Collection::new("Users");
        c.push(r);

        let text = yaml_serde::to_string(&c).unwrap();
        assert!(text.contains("secret:api_token"));
    }
}
