//! # rl-model
//!
//! The unified API model. Every source RouteLens understands — source code, cURL, OpenAPI,
//! raw HTTP, manual entry — resolves into the types here, so nothing downstream needs to
//! know where a request came from.
//!
//! ```text
//! Source Code ──┐
//! cURL ─────────┤
//! OpenAPI ──────┼──→  rl-model  ──→  UI · HTTP engine · storage
//! Raw HTTP ─────┤
//! Manual ───────┘
//! ```
//!
//! This crate performs no I/O and depends on no other RouteLens crate. Everything in it is
//! data and pure functions, which keeps it exhaustively testable and keeps the type
//! definitions honest.
//!
//! ## The two core types
//!
//! [`EndpointSpec`] is a route *discovered* from a project: a path shape, a schema, and a
//! source location, with no host and no values.
//!
//! [`RequestDraft`] is a request you can *send*: concrete values and a real URL, with no
//! source location.
//!
//! They are separate on purpose. Collapsing them looks tempting early and costs later —
//! history entries acquire meaningless source fields, discovered routes acquire meaningless
//! value fields, and every consumer has to check which kind it actually holds.
//!
//! ## Admitting what is unknown
//!
//! [`PathSegment::Unresolved`] carries the source expression that defeated static analysis,
//! so a path RouteLens could not work out is displayed as a gap rather than guessed at. A
//! confidently wrong path is worse than a missing one, because it fails in a way the
//! developer will blame on their own code.

pub mod draft;
pub mod method;
pub mod path;
pub mod spec;
pub mod vars;

pub use draft::{
    AuthConfig, BodyValue, FormPart, KeyValue, RequestDraft, RequestId, RequestSettings,
};
pub use method::HttpMethod;
pub use path::{ParamStyle, PathSegment, PathTemplate, TypeHint};
pub use spec::{
    ApiKeyLocation, AuthRequirement, BodySchema, Confidence, EndpointId, EndpointSpec, Origin,
    ParamLocation, ParamSpec, SourceLocation,
};
pub use vars::{ResolveError, Resolved, VariableContext, MAX_DEPTH, REDACTION};

#[cfg(test)]
mod integration {
    //! Cross-module behaviour: the paths a real request actually takes through the model.

    use super::*;

    /// The FastAPI shape from `docs/discovery/how-it-works.md`:
    /// a router declared with a prefix in one file, mounted with another prefix elsewhere.
    #[test]
    fn discovered_endpoint_becomes_a_sendable_request() {
        let mount = PathTemplate::parse("/api/v1", ParamStyle::Braces);
        let router = PathTemplate::parse("/users", ParamStyle::Braces);
        let route = PathTemplate::parse("/{user_id}", ParamStyle::Braces);
        let full = mount.join(&router).join(&route);

        let spec = EndpointSpec::new(
            HttpMethod::Get,
            full,
            Origin::StaticScan {
                framework: "fastapi".into(),
            },
        )
        .with_source(SourceLocation::new("api/users.py", 17))
        .with_group("users");

        assert_eq!(spec.display(), "GET /api/v1/users/{user_id}");

        let mut draft = RequestDraft::from_spec(&spec, "{{base_url}}");
        draft.path_values.insert("user_id".into(), "42".into());
        draft.auth = AuthConfig::Bearer {
            token: "{{secret:api_token}}".into(),
        };

        let mut ctx = VariableContext::new();
        ctx.environment
            .insert("base_url".into(), "http://localhost:8000".into());
        ctx.secrets.insert("api_token".into(), "s3cr3t".into());

        let (resolved, secrets_used) = draft.resolve(&ctx).unwrap();
        assert_eq!(
            resolved.url_with_path_values(),
            "http://localhost:8000/api/v1/users/42"
        );
        assert!(secrets_used.contains("api_token"));
    }

    /// A secret must survive round-tripping as a *reference*, and must never be written into
    /// anything that could be committed. This is the test that must never regress.
    #[test]
    fn secrets_never_reach_serialized_output() {
        let mut ctx = VariableContext::new();
        ctx.secrets.insert("api_token".into(), "s3cr3t".into());

        let mut draft = RequestDraft::new(HttpMethod::Get, "https://api.example.com/me");
        draft.auth = AuthConfig::Bearer {
            token: "{{secret:api_token}}".into(),
        };

        // What gets saved keeps the reference.
        let saved = serde_json::to_string(&draft).unwrap();
        assert!(saved.contains("{{secret:api_token}}"));
        assert!(!saved.contains("s3cr3t"));

        // What gets sent has the value.
        let (resolved, secrets_used) = draft.resolve(&ctx).unwrap();
        assert_eq!(
            resolved.auth,
            AuthConfig::Bearer {
                token: "s3cr3t".into()
            }
        );

        // What gets written to history has neither.
        let sent = serde_json::to_string(&resolved).unwrap();
        let for_history = ctx.redact(&sent, &secrets_used);
        assert!(!for_history.contains("s3cr3t"));
        assert!(for_history.contains(REDACTION));
    }

    /// Two adapters describing the same route in different syntaxes must agree on identity,
    /// so the static and runtime merge in P5 can key on it.
    #[test]
    fn the_same_route_from_two_sources_shares_an_id() {
        let from_source = EndpointSpec::new(
            HttpMethod::Get,
            PathTemplate::parse("/users/:id", ParamStyle::Colon),
            Origin::StaticScan {
                framework: "express".into(),
            },
        );
        let from_openapi = EndpointSpec::new(
            HttpMethod::Get,
            PathTemplate::parse("/users/{userId}", ParamStyle::Braces),
            Origin::OpenApi { document: None },
        );
        assert_eq!(from_source.id, from_openapi.id);
    }

    /// An unmounted router is kept and flagged, never dropped — it is usually a bug in the
    /// project being inspected, and saying so is more useful than staying silent.
    #[test]
    fn an_orphaned_route_is_reported_rather_than_hidden() {
        let mut spec = EndpointSpec::new(
            HttpMethod::Get,
            PathTemplate::parse("/orphan", ParamStyle::Braces),
            Origin::StaticScan {
                framework: "express".into(),
            },
        );
        spec.orphaned = true;
        assert!(spec.has_gaps());
    }
}
