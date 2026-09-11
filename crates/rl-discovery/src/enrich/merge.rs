//! Union of what the source says and what the application says.
//!
//! Neither list replaces the other. Keyed on `(method, normalized path)`, each endpoint is
//! one of:
//!
//! | Found by | Kept as | Why |
//! |---|---|---|
//! | both | one merged spec | runtime fields, static source location |
//! | static only | flagged `static_only` | probably unreachable — a dead router, an orphan |
//! | runtime only | flagged `runtime_only` | usually the dynamic registrations static analysis is blind to |
//! | static with a gap, runtime fills it | merged, flagged `gap_filled` | `/?/stats` learned it is `/admin/stats` |
//!
//! Per field, runtime wins on everything it actually knows — parameters, schemas, auth,
//! grouping — and static wins on the one thing runtime cannot know: where the code is.
//! The differences between the two lists are the informative part, which is why they are
//! flagged rather than smoothed over.

use rl_model::{Confidence, EndpointSpec, PathSegment};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The metadata key every enriched spec carries.
pub const PROVENANCE_KEY: &str = "enrich";

/// How a spec fared in the merge; the value under [`PROVENANCE_KEY`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Found by both; runtime detail on a static location.
    Matched,
    /// Found only in source: probably unreachable.
    StaticOnly,
    /// Found only by asking the application: no source location.
    RuntimeOnly,
    /// A static path with an unresolved segment that runtime resolved.
    GapFilled,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::Matched => "matched",
            Provenance::StaticOnly => "static_only",
            Provenance::RuntimeOnly => "runtime_only",
            Provenance::GapFilled => "gap_filled",
        }
    }

    pub fn of(spec: &EndpointSpec) -> Option<Provenance> {
        match spec.metadata.get(PROVENANCE_KEY)?.as_str()? {
            "matched" => Some(Provenance::Matched),
            "static_only" => Some(Provenance::StaticOnly),
            "runtime_only" => Some(Provenance::RuntimeOnly),
            "gap_filled" => Some(Provenance::GapFilled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeReport {
    pub matched: usize,
    pub static_only: usize,
    pub runtime_only: usize,
    pub gaps_filled: usize,
}

/// Merge runtime specs into static ones. Both lists are consumed; the result is sorted the
/// way a scan sorts.
pub fn merge(
    static_specs: Vec<EndpointSpec>,
    runtime: Vec<EndpointSpec>,
) -> (Vec<EndpointSpec>, MergeReport) {
    let mut report = MergeReport::default();
    let mut out = Vec::with_capacity(static_specs.len().max(runtime.len()));

    // Runtime specs by identity, consumed as they are claimed.
    let mut unclaimed: BTreeMap<String, EndpointSpec> = runtime
        .into_iter()
        .map(|spec| (spec.id.as_str().to_string(), spec))
        .collect();

    let mut with_gaps = Vec::new();
    let mut unmatched = Vec::new();
    for spec in static_specs {
        match unclaimed.remove(spec.id.as_str()) {
            Some(runtime) => {
                report.matched += 1;
                out.push(combine(spec, runtime, Provenance::Matched));
            }
            None if !spec.path.is_resolved() => with_gaps.push(spec),
            None => unmatched.push(spec),
        }
    }

    // A catch-all read from source (`<path:name>`, `{name:path}`) is a plain parameter in
    // an OpenAPI document that did not say otherwise; the two must still be one route.
    let mut claimed: Vec<EndpointSpec> = Vec::new();
    for spec in unmatched {
        let loose = loose_key(&spec);
        let found = unclaimed
            .iter()
            .find(|(_, r)| loose_key(r) == loose)
            .map(|(id, _)| id.clone());
        if let Some(id) = found {
            let runtime = unclaimed.remove(&id).expect("just found");
            report.matched += 1;
            claimed.push(runtime.clone());
            out.push(combine(spec, runtime, Provenance::Matched));
            continue;
        }

        // The application listed this path but could not say which methods the view
        // takes — a plain Django function view. The source said `POST`; believe it.
        let path = loose_path(&spec);
        let sibling = unclaimed
            .values()
            .chain(claimed.iter())
            .chain(out.iter())
            .find(|r| loose_path(r) == path && methods_unknown(r))
            .cloned();
        match sibling {
            Some(mut runtime) => {
                runtime.method = spec.method.clone();
                runtime.id = rl_model::EndpointId::new(&runtime.method, &runtime.path);
                report.matched += 1;
                out.push(combine(spec, runtime, Provenance::Matched));
            }
            None => out.push(mark(spec, Provenance::StaticOnly)),
        }
    }

    // A second pass for unresolved static paths: `/?/stats` matches exactly one remaining
    // runtime route ending in `/stats`, and that closes the gap. Ambiguity leaves it open.
    for spec in with_gaps {
        let candidates: Vec<String> = unclaimed
            .values()
            .filter(|r| r.method == spec.method && fills_gap(&spec, r))
            .map(|r| r.id.as_str().to_string())
            .collect();
        match candidates.as_slice() {
            [only] => {
                let runtime = unclaimed.remove(only).expect("just listed");
                report.gaps_filled += 1;
                let exprs: Vec<String> = spec
                    .path
                    .unresolved_exprs()
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                let mut merged = combine(spec, runtime, Provenance::GapFilled);
                merged.metadata.insert(
                    "resolved_from".into(),
                    serde_json::Value::from(exprs.join(", ")),
                );
                out.push(merged);
            }
            _ => out.push(mark(spec, Provenance::StaticOnly)),
        }
    }

    for (_, runtime) in unclaimed {
        report.runtime_only += 1;
        out.push(mark(runtime, Provenance::RuntimeOnly));
    }

    report.static_only = out
        .iter()
        .filter(|s| Provenance::of(s) == Some(Provenance::StaticOnly))
        .count();

    out.sort_by(|a, b| {
        a.path
            .normalized()
            .cmp(&b.path.normalized())
            .then_with(|| a.method.as_str().cmp(b.method.as_str()))
    });
    (out, report)
}

/// Identity with catch-all parameters flattened to plain ones.
fn loose_key(spec: &EndpointSpec) -> String {
    format!("{} {}", spec.method.as_str(), loose_path(spec))
}

fn loose_path(spec: &EndpointSpec) -> String {
    spec.path.normalized().replace("{*}", "{}")
}

/// The generator listed the path but not its methods — runtime enrich's
/// `x-methods-unknown` on a plain Django function view.
fn methods_unknown(spec: &EndpointSpec) -> bool {
    spec.metadata
        .get("methods-unknown")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

/// Runtime detail onto a static location.
fn combine(
    static_spec: EndpointSpec,
    runtime: EndpointSpec,
    provenance: Provenance,
) -> EndpointSpec {
    let mut merged = runtime;
    merged.source = static_spec.source;
    merged.group = merged.group.or(static_spec.group);
    merged.summary = merged.summary.or(static_spec.summary);
    merged.description = merged.description.or(static_spec.description);
    // The application serves it, so it is reachable whatever the graph thought.
    merged.orphaned = false;
    merged.confidence = Confidence::High;
    for (key, value) in static_spec.metadata {
        merged.metadata.entry(key).or_insert(value);
    }
    mark(merged, provenance)
}

fn mark(mut spec: EndpointSpec, provenance: Provenance) -> EndpointSpec {
    spec.metadata.insert(
        PROVENANCE_KEY.into(),
        serde_json::Value::from(provenance.as_str()),
    );
    if provenance == Provenance::StaticOnly {
        // The application was asked and did not list it.
        spec.confidence = Confidence::Low;
    }
    spec
}

/// Whether a runtime path could be what an unresolved static path stands for.
///
/// The resolved segments on either side of the gap must line up: everything before the
/// first unresolved segment is a prefix of the runtime path, everything after the last one
/// is a suffix, and the gap covers at least one runtime segment.
fn fills_gap(static_spec: &EndpointSpec, runtime: &EndpointSpec) -> bool {
    let segments = &static_spec.path.segments;
    let first_gap = segments
        .iter()
        .position(|s| matches!(s, PathSegment::Unresolved { .. }));
    let last_gap = segments
        .iter()
        .rposition(|s| matches!(s, PathSegment::Unresolved { .. }));
    let (Some(first), Some(last)) = (first_gap, last_gap) else {
        return false;
    };

    let head = &segments[..first];
    let tail = &segments[last + 1..];
    let actual = &runtime.path.segments;
    if actual.len() < head.len() + tail.len() + 1 {
        return false;
    }

    let same = |a: &PathSegment, b: &PathSegment| match (a, b) {
        (PathSegment::Literal { value: x }, PathSegment::Literal { value: y }) => x == y,
        (PathSegment::Param { .. }, PathSegment::Param { .. }) => true,
        _ => false,
    };
    head.iter().zip(actual.iter()).all(|(a, b)| same(a, b))
        && tail
            .iter()
            .rev()
            .zip(actual.iter().rev())
            .all(|(a, b)| same(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_model::{HttpMethod, Origin, ParamStyle, PathTemplate, SourceLocation};

    fn static_spec(method: HttpMethod, path: &str) -> EndpointSpec {
        EndpointSpec::new(
            method,
            PathTemplate::parse(path, ParamStyle::Braces),
            Origin::StaticScan {
                framework: "fastapi".into(),
            },
        )
        .with_source(SourceLocation::new("app/main.py", 7))
    }

    fn runtime_spec(method: HttpMethod, path: &str) -> EndpointSpec {
        EndpointSpec::new(
            method,
            PathTemplate::parse(path, ParamStyle::Braces),
            Origin::Runtime {
                framework: "fastapi".into(),
            },
        )
    }

    fn unresolved(method: HttpMethod, expr: &str, tail: &[&str]) -> EndpointSpec {
        let mut segments = vec![PathSegment::unresolved(expr)];
        segments.extend(tail.iter().map(|t| PathSegment::literal(*t)));
        EndpointSpec::new(
            method,
            PathTemplate::from_segments(segments),
            Origin::StaticScan {
                framework: "fastapi".into(),
            },
        )
        .with_source(SourceLocation::new("app/admin.py", 3))
    }

    #[test]
    fn a_route_found_by_both_keeps_the_source_and_takes_runtime_detail() {
        let mut runtime = runtime_spec(HttpMethod::Get, "/users/{id}");
        runtime.summary = Some("List".into());
        runtime.group = Some("users".into());

        let (merged, report) = merge(
            vec![static_spec(HttpMethod::Get, "/users/{user_id}")],
            vec![runtime],
        );

        assert_eq!(report.matched, 1);
        let spec = &merged[0];
        assert_eq!(spec.source.as_ref().unwrap().line, 7);
        assert_eq!(spec.summary.as_deref(), Some("List"));
        assert_eq!(spec.confidence, Confidence::High);
        assert!(matches!(spec.origin, Origin::Runtime { .. }));
        assert_eq!(Provenance::of(spec), Some(Provenance::Matched));
    }

    #[test]
    fn a_static_only_route_is_kept_flagged_and_demoted() {
        let mut orphan = static_spec(HttpMethod::Get, "/orphan");
        orphan.orphaned = true;
        let (merged, report) = merge(vec![orphan], vec![]);

        assert_eq!(report.static_only, 1);
        assert_eq!(Provenance::of(&merged[0]), Some(Provenance::StaticOnly));
        assert_eq!(merged[0].confidence, Confidence::Low);
        assert!(merged[0].orphaned, "the static flags survive");
    }

    #[test]
    fn a_runtime_only_route_is_kept_without_a_source() {
        let (merged, report) = merge(vec![], vec![runtime_spec(HttpMethod::Get, "/dyn/widgets")]);
        assert_eq!(report.runtime_only, 1);
        assert!(merged[0].source.is_none());
        assert_eq!(Provenance::of(&merged[0]), Some(Provenance::RuntimeOnly));
    }

    #[test]
    fn a_gap_is_filled_when_exactly_one_runtime_route_fits() {
        let (merged, report) = merge(
            vec![unresolved(
                HttpMethod::Get,
                "settings.API_PREFIX",
                &["stats"],
            )],
            vec![
                runtime_spec(HttpMethod::Get, "/admin/stats"),
                runtime_spec(HttpMethod::Get, "/health"),
            ],
        );

        assert_eq!(report.gaps_filled, 1);
        assert_eq!(report.runtime_only, 1);
        let filled = merged
            .iter()
            .find(|s| Provenance::of(s) == Some(Provenance::GapFilled))
            .unwrap();
        assert_eq!(filled.path.to_string(), "/admin/stats");
        assert!(filled.path.is_resolved());
        assert_eq!(
            filled.source.as_ref().unwrap().line,
            3,
            "source comes from static"
        );
        assert_eq!(
            filled
                .metadata
                .get("resolved_from")
                .and_then(|v| v.as_str()),
            Some("settings.API_PREFIX")
        );
    }

    #[test]
    fn an_ambiguous_gap_stays_open() {
        let (merged, report) = merge(
            vec![unresolved(HttpMethod::Get, "PREFIX", &["stats"])],
            vec![
                runtime_spec(HttpMethod::Get, "/admin/stats"),
                runtime_spec(HttpMethod::Get, "/internal/stats"),
            ],
        );
        assert_eq!(report.gaps_filled, 0);
        assert_eq!(report.runtime_only, 2);
        assert!(merged.iter().any(|s| !s.path.is_resolved()));
    }

    #[test]
    fn a_gap_needs_the_method_and_the_tail_to_agree() {
        let (_, report) = merge(
            vec![unresolved(HttpMethod::Post, "PREFIX", &["stats"])],
            vec![runtime_spec(HttpMethod::Get, "/admin/stats")],
        );
        assert_eq!(report.gaps_filled, 0);

        let (_, report) = merge(
            vec![unresolved(HttpMethod::Get, "PREFIX", &["stats"])],
            vec![runtime_spec(HttpMethod::Get, "/admin/stats/daily")],
        );
        assert_eq!(report.gaps_filled, 0, "the tail must be a suffix");
    }

    #[test]
    fn a_catch_all_matches_a_plain_parameter_from_the_document() {
        let (merged, report) = merge(
            vec![static_spec(HttpMethod::Get, "/files/{name:path}")],
            vec![runtime_spec(HttpMethod::Get, "/files/{name}")],
        );
        assert_eq!(report.matched, 1);
        assert_eq!(report.runtime_only, 0);
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn a_static_method_survives_when_the_application_did_not_know_its_methods() {
        let mut runtime = runtime_spec(HttpMethod::Get, "/items/");
        runtime
            .metadata
            .insert("methods-unknown".into(), serde_json::Value::Bool(true));
        let (merged, report) = merge(
            vec![
                static_spec(HttpMethod::Get, "/items/"),
                static_spec(HttpMethod::Post, "/items/"),
            ],
            vec![runtime],
        );
        assert_eq!(report.matched, 2, "{merged:?}");
        assert_eq!(report.static_only, 0);
        let post = merged
            .iter()
            .find(|s| s.method == HttpMethod::Post)
            .unwrap();
        assert_eq!(Provenance::of(post), Some(Provenance::Matched));
        assert!(post.source.is_some());
    }

    #[test]
    fn trailing_slashes_do_not_split_an_identity() {
        let (_, report) = merge(
            vec![static_spec(HttpMethod::Get, "/users/")],
            vec![runtime_spec(HttpMethod::Get, "/users")],
        );
        assert_eq!(report.matched, 1);
    }
}
