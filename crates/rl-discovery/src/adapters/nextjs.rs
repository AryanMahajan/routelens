//! Next.js.
//!
//! The cheapest adapter, because the path is the file system: `app/api/users/[id]/route.ts`
//! *is* `/api/users/[id]`. The syntax tree is only needed for which methods a route handler
//! exports, and — for the legacy `pages/api` router — which methods a handler checks for.
//!
//! There is no router to mount, so every route file registers on one virtual root per app,
//! attached through a pre-resolved import (see [`crate::facts::ImportFact::source`]).

use super::js::{
    arguments, auth_from_middleware, callee, string_literal, top_level_function, RequestUsage,
};
use super::{Detection, FrameworkAdapter, UNSPECIFIED_METHODS};
use crate::facts::{FactSink, ImportFact, RouteFact, RouterFact, Span, SymbolId, SymbolRef};
use crate::index::{walk, ParsedFile};
use crate::project::{Language, ProjectContext};
use rl_model::{AuthRequirement, BodySchema, HttpMethod, ParamSpec, ParamStyle, PathTemplate};
use std::path::{Path, PathBuf};
use tree_sitter::Node;

/// The methods a route handler may export, by Next's own rule: uppercase, these seven.
const HANDLER_METHODS: &[(&str, HttpMethod)] = &[
    ("GET", HttpMethod::Get),
    ("POST", HttpMethod::Post),
    ("PUT", HttpMethod::Put),
    ("PATCH", HttpMethod::Patch),
    ("DELETE", HttpMethod::Delete),
    ("HEAD", HttpMethod::Head),
    ("OPTIONS", HttpMethod::Options),
];

const ROUTE_FILE_STEMS: &[&str] = &["route"];

pub struct NextJsAdapter;

/// Which of Next's two routers a file belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// `app/**/route.ts` — the App Router's route handlers.
    App,
    /// `pages/api/**.ts` — the Pages Router's API routes.
    PagesApi,
}

/// A route file, located.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Located {
    kind: Kind,
    /// The directory the router lives under: `.`, `src`, `apps/web`.
    root: PathBuf,
    path: PathTemplate,
}

impl FrameworkAdapter for NextJsAdapter {
    fn id(&self) -> &'static str {
        "nextjs"
    }

    fn languages(&self) -> &[Language] {
        &[Language::JavaScript, Language::TypeScript]
    }

    fn detect(&self, project: &ProjectContext) -> Detection {
        let mut detection = Detection::none();

        if project
            .manifest("package.json")
            .is_some_and(|text| text.contains("\"next\""))
        {
            detection.add(2, "`next` is declared in package.json");
        }
        for name in ["next.config.js", "next.config.mjs", "next.config.ts"] {
            if project.has_manifest(name) {
                detection.add(2, format!("{name} is present"));
                break;
            }
        }

        // Route files are the framework's own convention, and the strongest evidence.
        let mut counts = (0, 0);
        for path in project.files() {
            match locate(path).map(|l| l.kind) {
                Some(Kind::App) => counts.0 += 1,
                Some(Kind::PagesApi) => counts.1 += 1,
                None => {}
            }
        }
        if counts.0 > 0 {
            detection.add(3, format!("{} route handler(s) under app/", counts.0));
        }
        if counts.1 > 0 {
            detection.add(3, format!("{} API route(s) under pages/api/", counts.1));
        }

        detection
    }

    fn candidate_files(&self, project: &ProjectContext) -> Vec<PathBuf> {
        project
            .files()
            .iter()
            .filter(|path| locate(path).is_some())
            .cloned()
            .collect()
    }

    fn extract(&self, file: &ParsedFile, sink: &mut FactSink) {
        let Some(located) = locate(&file.path) else {
            return;
        };

        // One virtual root per app directory. Re-declared by every file; the graph keeps
        // the first, and nothing in it is file-specific.
        let root = SymbolId::new(located.root.clone(), "next");
        sink.router(RouterFact {
            symbol: root.clone(),
            prefix: PathTemplate::empty(),
            group: None,
            is_app_root: true,
            span: Span::default(),
        });
        sink.import(ImportFact {
            module: file.path.clone(),
            local_name: "next".to_string(),
            source: format!("/{}", crate::project::display(&located.root)),
            original: Some("next".to_string()),
            level: 0,
        });

        let group = group_of(&located.path);
        let router = SymbolRef::new(file.path.clone(), "next");

        match located.kind {
            Kind::App => {
                let handlers = method_exports(file);
                if handlers.is_empty() {
                    sink.warn(format!(
                        "{} exports no HTTP method handler (GET, POST, …)",
                        file.display_path()
                    ));
                }
                for (method, span, function) in handlers {
                    let mut fact =
                        RouteFact::new(router.clone(), method, located.path.clone(), span);
                    fact.group = group.clone();
                    if let Some(function) = function {
                        apply_handler_usage(file, function, &mut fact);
                    }
                    super::js::promote_authorization_header(&mut fact.headers, &mut fact.auth);
                    sink.route(fact);
                }
            }
            Kind::PagesApi => {
                let Some(handler) = default_export(file) else {
                    sink.warn(format!(
                        "{} has no default export, so it is not an API route",
                        file.display_path()
                    ));
                    return;
                };

                let methods = checked_methods(file);
                let unspecified = methods.is_empty();
                let mut fact = RouteFact::new(
                    router.clone(),
                    HttpMethod::Get,
                    located.path.clone(),
                    handler.span,
                );
                fact.methods = if unspecified {
                    UNSPECIFIED_METHODS.to_vec()
                } else {
                    methods
                };
                fact.group = group;
                fact.auth = handler.auth;
                if unspecified {
                    fact.summary = Some(
                        "the handler never checks `req.method`, so any method reaches it".into(),
                    );
                }
                if let Some(function) = handler.function {
                    let usage = RequestUsage::of(file, function);
                    // The Pages Router puts dynamic segments in `req.query` too, so
                    // `req.query.id` under `[id].ts` is the path parameter, not a query.
                    let path_params = located.path.param_names();
                    fact.query_params = usage
                        .query
                        .iter()
                        .filter(|n| !path_params.contains(&n.as_str()))
                        .map(ParamSpec::new)
                        .collect();
                    fact.headers = usage.headers.iter().map(ParamSpec::new).collect();
                    if usage.reads_body {
                        fact.body = Some(BodySchema::json());
                    }
                }
                super::js::promote_authorization_header(&mut fact.headers, &mut fact.auth);
                sink.route(fact);
            }
        }

        if file.has_errors() {
            sink.warn(format!(
                "{} has syntax errors; its handlers may be missing",
                file.display_path()
            ));
        }
    }
}

/// Work out whether a file is a route, and what path the file system gives it.
fn locate(path: &Path) -> Option<Located> {
    let components: Vec<&str> = path.iter().filter_map(|c| c.to_str()).collect();
    let (file, dirs) = components.split_last()?;
    let (stem, extension) = file.rsplit_once('.')?;
    if !matches!(extension, "js" | "jsx" | "ts" | "tsx" | "mjs") {
        return None;
    }

    // App Router: the first `app` directory, so `app/api/app/route.ts` still reads as
    // `/api/app` rather than being cut at the second `app`.
    if ROUTE_FILE_STEMS.contains(&stem) {
        if let Some(index) = dirs.iter().position(|d| *d == "app") {
            let segments = &dirs[index + 1..];
            let mut parts = Vec::new();
            for segment in segments {
                // `_lib/` opts a folder out of routing entirely.
                if segment.starts_with('_') {
                    return None;
                }
                // `(marketing)/` groups without affecting the URL; `@modal/` is a parallel
                // slot; `(.)photo/` intercepts, and the marker is not part of the path.
                if segment.starts_with('@') || is_group(segment) {
                    continue;
                }
                let segment = strip_intercept_marker(segment);
                parts.push(segment);
            }
            return Some(Located {
                kind: Kind::App,
                root: root_of(&dirs[..index]),
                path: PathTemplate::parse(&parts.join("/"), ParamStyle::Bracket),
            });
        }
        return None;
    }

    // Pages Router: `pages/api/**`. Files and folders starting with `_` are not routes.
    if stem.ends_with(".d") {
        return None;
    }
    let index = dirs
        .windows(2)
        .position(|w| w[0] == "pages" && w[1] == "api")?;
    let segments = &dirs[index + 1..];
    if segments.iter().any(|s| s.starts_with('_')) || stem.starts_with('_') {
        return None;
    }
    let mut parts: Vec<&str> = segments.to_vec();
    if stem != "index" {
        parts.push(stem);
    }
    Some(Located {
        kind: Kind::PagesApi,
        root: root_of(&dirs[..index]),
        path: PathTemplate::parse(&parts.join("/"), ParamStyle::Bracket),
    })
}

fn is_group(segment: &str) -> bool {
    segment.starts_with('(') && segment.ends_with(')')
}

/// `(.)photo` → `photo`, `(..)(..)photo` → `photo`.
fn strip_intercept_marker(segment: &str) -> &str {
    let mut rest = segment;
    while rest.starts_with('(') {
        match rest.find(')') {
            Some(end) => rest = &rest[end + 1..],
            None => break,
        }
    }
    rest
}

/// The directory the router lives under, with `src/` folded in so `src/app` and `app`
/// belong to the same app.
fn root_of(dirs: &[&str]) -> PathBuf {
    let dirs = match dirs.split_last() {
        Some((last, rest)) if *last == "src" => rest,
        _ => dirs,
    };
    if dirs.is_empty() {
        PathBuf::from(".")
    } else {
        dirs.iter().collect()
    }
}

/// `/api/users/[id]` → `users`: the first literal after `api`, or the first literal.
fn group_of(path: &PathTemplate) -> Option<String> {
    let literals: Vec<&str> = path
        .segments
        .iter()
        .filter_map(|s| match s {
            rl_model::PathSegment::Literal { value } => Some(value.as_str()),
            _ => None,
        })
        .collect();
    let after_api = literals
        .iter()
        .position(|l| *l == "api")
        .map(|i| &literals[i + 1..])
        .unwrap_or(&literals[..]);
    after_api.first().map(|s| s.to_string())
}

/// The method handlers a route file exports, each with where it was exported and — when it
/// can be found — the function itself.
fn method_exports<'a>(file: &'a ParsedFile) -> Vec<(HttpMethod, Span, Option<Node<'a>>)> {
    let mut found = Vec::new();
    let mut seen = Vec::new();

    let mut record = |name: &str, span: Span, function: Option<Node<'a>>| {
        if let Some((_, method)) = HANDLER_METHODS.iter().find(|(n, _)| *n == name) {
            if !seen.contains(&method) {
                seen.push(method);
                found.push((method.clone(), span, function));
            }
        }
    };

    walk(file.root(), &mut |node| {
        if node.kind() != "export_statement" {
            return;
        }
        let span = file.span(node);

        // `export { GET, handler as POST } from "./shared"` and `export { GET }`
        if let Some(clause) = crate::index::child_of_kind(node, "export_clause") {
            let re_export = node.child_by_field_name("source").is_some();
            let mut cursor = clause.walk();
            for specifier in clause.named_children(&mut cursor) {
                let Some(name) = specifier.child_by_field_name("name") else {
                    continue;
                };
                let exported = specifier.child_by_field_name("alias").unwrap_or(name);
                let function = if re_export {
                    None
                } else {
                    top_level_function(file, file.text(name))
                };
                record(file.text(exported), span, function);
            }
            return;
        }

        let Some(declaration) = node.child_by_field_name("declaration") else {
            return;
        };
        match declaration.kind() {
            // `export async function GET(request) {}`
            "function_declaration" => {
                if let Some(name) = declaration.child_by_field_name("name") {
                    record(file.text(name), span, Some(declaration));
                }
            }
            // `export const GET = async () => {}`, `export const POST = handler`,
            // `export const { PUT, PATCH } = handlers`
            "lexical_declaration" | "variable_declaration" => {
                let mut cursor = declaration.walk();
                for declarator in declaration.named_children(&mut cursor) {
                    let (Some(name), value) = (
                        declarator.child_by_field_name("name"),
                        declarator.child_by_field_name("value"),
                    ) else {
                        continue;
                    };
                    match name.kind() {
                        "identifier" => {
                            let function = match value {
                                Some(v) if super::js::is_function(v) => Some(v),
                                Some(v) if v.kind() == "identifier" => {
                                    top_level_function(file, file.text(v))
                                }
                                _ => None,
                            };
                            record(file.text(name), span, function);
                        }
                        "object_pattern" => {
                            for entry in super::js::declared_names(file, declaration) {
                                record(&entry, span, None);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    });

    found
}

/// What a route handler reads: `searchParams.get("q")`, `request.json()`,
/// `request.headers.get("x-api-key")`.
fn apply_handler_usage(file: &ParsedFile, function: Node<'_>, fact: &mut RouteFact) {
    let request = super::js::first_parameter_name(file, function);
    let Some(body) = function.child_by_field_name("body") else {
        return;
    };

    walk(body, &mut |node| {
        if node.kind() != "call_expression" {
            return;
        }
        let Some((Some(receiver), method)) = callee(file, node) else {
            return;
        };
        let receiver_text = file.text(receiver);
        let first_string = arguments(node)
            .first()
            .filter(|a| a.kind() == "string")
            .map(|a| string_literal(file, *a));

        if receiver_text.ends_with("searchParams")
            && matches!(method.as_str(), "get" | "getAll" | "has")
        {
            if let Some(name) = first_string {
                if !fact.query_params.iter().any(|p| p.name == name) {
                    fact.query_params.push(ParamSpec::new(name));
                }
            }
            return;
        }

        if receiver_text.ends_with("headers") && method == "get" {
            if let Some(name) = first_string {
                if !fact.headers.iter().any(|p| p.name == name) {
                    fact.headers.push(ParamSpec::new(name));
                }
            }
            return;
        }

        if request.as_deref() == Some(receiver_text) && fact.body.is_none() {
            fact.body = match method.as_str() {
                "json" => Some(BodySchema::json()),
                "formData" => Some(BodySchema {
                    content_type: "multipart/form-data".into(),
                    schema: None,
                    example: None,
                    required: false,
                }),
                "text" => Some(BodySchema {
                    content_type: "text/plain".into(),
                    schema: None,
                    example: None,
                    required: false,
                }),
                _ => None,
            };
        }
    });
}

/// A `pages/api` file's default export, as far as it can be followed.
struct DefaultExport<'a> {
    span: Span,
    function: Option<Node<'a>>,
    /// `export default withAuth(handler)` — the wrapper's name is the only hint there is.
    auth: Option<AuthRequirement>,
}

fn default_export(file: &ParsedFile) -> Option<DefaultExport<'_>> {
    let mut found = None;
    walk(file.root(), &mut |node| {
        if found.is_some() || node.kind() != "export_statement" {
            return;
        }
        let is_default = (0..node.child_count() as u32)
            .filter_map(|i| node.child(i))
            .any(|c| c.kind() == "default");
        if !is_default {
            return;
        }
        let span = file.span(node);

        if let Some(declaration) = node.child_by_field_name("declaration") {
            // `export default function handler(req, res) {}`
            found = Some(DefaultExport {
                span,
                function: super::js::is_function(declaration).then_some(declaration),
                auth: None,
            });
            return;
        }

        let Some(value) = node.child_by_field_name("value") else {
            return;
        };
        let (function, auth) = match value.kind() {
            "identifier" => (top_level_function(file, file.text(value)), None),
            "call_expression" => {
                // `export default withAuth(handler)`: inspect the wrapped handler when it
                // is named, and take the wrapper as an auth hint.
                let wrapper = value
                    .child_by_field_name("function")
                    .map(|f| file.text(f).to_string())
                    .unwrap_or_default();
                let inner = arguments(value).into_iter().find_map(|a| {
                    if super::js::is_function(a) {
                        Some(a)
                    } else if a.kind() == "identifier" {
                        top_level_function(file, file.text(a))
                    } else {
                        None
                    }
                });
                (inner, auth_from_middleware(&wrapper))
            }
            _ if super::js::is_function(value) => (Some(value), None),
            _ => (None, None),
        };
        found = Some(DefaultExport {
            span,
            function,
            auth,
        });
    });
    found
}

/// The methods a `pages/api` handler compares `req.method` against.
///
/// `req.method === "POST"`, `req.method !== "POST"` (a guard is still a statement of what
/// is accepted), `switch (req.method) { case "GET": }` and
/// `["GET", "POST"].includes(req.method)`.
fn checked_methods(file: &ParsedFile) -> Vec<HttpMethod> {
    let mut methods: Vec<HttpMethod> = Vec::new();
    let mut push = |text: &str| {
        // `parse` never fails — an unknown string becomes `Other` — and a comparison
        // against `"banana"` is not a method check.
        let Ok(method) = text.parse::<HttpMethod>();
        if matches!(method, HttpMethod::Other(_)) {
            return;
        }
        if !methods.contains(&method) {
            methods.push(method);
        }
    };

    walk(file.root(), &mut |node| match node.kind() {
        "binary_expression" => {
            let (Some(left), Some(right)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) else {
                return;
            };
            let (subject, literal) = if right.kind() == "string" {
                (left, right)
            } else if left.kind() == "string" {
                (right, left)
            } else {
                return;
            };
            if file.text(subject).ends_with(".method") {
                push(&string_literal(file, literal));
            }
        }
        "switch_statement" => {
            let Some(value) = node.child_by_field_name("value") else {
                return;
            };
            if !file.text(value).contains(".method") {
                return;
            }
            walk(node, &mut |case| {
                if case.kind() == "switch_case" {
                    if let Some(v) = case.child_by_field_name("value") {
                        if v.kind() == "string" {
                            push(&string_literal(file, v));
                        }
                    }
                }
            });
        }
        "call_expression" => {
            let Some((Some(receiver), method)) = callee(file, node) else {
                return;
            };
            if method != "includes" || receiver.kind() != "array" {
                return;
            }
            let checks_method = arguments(node)
                .first()
                .is_some_and(|a| file.text(*a).ends_with(".method"));
            if !checks_method {
                return;
            }
            let mut cursor = receiver.walk();
            for element in receiver.named_children(&mut cursor) {
                if element.kind() == "string" {
                    push(&string_literal(file, element));
                }
            }
        }
        _ => {}
    });

    methods
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::SourceIndex;

    fn located(path: &str) -> Option<(Kind, String, String)> {
        locate(Path::new(path)).map(|l| {
            (
                l.kind,
                crate::project::display(&l.root),
                l.path.render(ParamStyle::Braces),
            )
        })
    }

    fn extract(path: &str, source: &str) -> FactSink {
        let language = Language::of(Path::new(path)).unwrap();
        let file = SourceIndex::new()
            .unwrap()
            .parse(path, language, source.to_string())
            .unwrap();
        let mut sink = FactSink::new();
        NextJsAdapter.extract(&file, &mut sink);
        sink
    }

    fn methods(sink: &FactSink) -> Vec<String> {
        sink.routes
            .iter()
            .flat_map(|r| r.methods.iter().map(|m| m.to_string()))
            .collect()
    }

    #[test]
    fn app_router_paths_come_from_the_file_system() {
        assert_eq!(
            located("app/api/users/route.ts"),
            Some((Kind::App, ".".into(), "/api/users".into()))
        );
        assert_eq!(
            located("src/app/api/users/[id]/route.ts"),
            Some((Kind::App, ".".into(), "/api/users/{id}".into()))
        );
        assert_eq!(
            located("apps/web/app/api/docs/[...slug]/route.js"),
            Some((Kind::App, "apps/web".into(), "/api/docs/{slug}".into()))
        );
        assert_eq!(
            located("app/api/search/[[...q]]/route.ts").unwrap().2,
            "/api/search/{q}"
        );
    }

    #[test]
    fn groups_slots_and_intercepts_do_not_appear_in_the_path() {
        assert_eq!(
            located("app/(admin)/api/stats/route.ts").unwrap().2,
            "/api/stats"
        );
        assert_eq!(
            located("app/api/@modal/(.)photo/route.ts").unwrap().2,
            "/api/photo"
        );
    }

    #[test]
    fn private_folders_and_non_route_files_are_not_routes() {
        assert_eq!(located("app/api/_internal/route.ts"), None);
        assert_eq!(located("app/api/users/page.tsx"), None);
        assert_eq!(located("app/api/users/handler.ts"), None);
        assert_eq!(located("lib/route.ts"), None);
        assert_eq!(located("app/api/users/route.test.ts"), None);
    }

    #[test]
    fn a_second_app_segment_is_part_of_the_path() {
        assert_eq!(located("app/api/app/route.ts").unwrap().2, "/api/app");
    }

    #[test]
    fn pages_api_paths_come_from_the_file_name() {
        assert_eq!(
            located("pages/api/hello.ts"),
            Some((Kind::PagesApi, ".".into(), "/api/hello".into()))
        );
        assert_eq!(located("pages/api/index.ts").unwrap().2, "/api");
        assert_eq!(
            located("src/pages/api/users/[id].ts").unwrap().2,
            "/api/users/{id}"
        );
        assert_eq!(
            located("pages/api/auth/[...nextauth].ts").unwrap().2,
            "/api/auth/{nextauth}"
        );
        assert_eq!(located("pages/api/_middleware.ts"), None);
        assert_eq!(located("pages/index.tsx"), None);
        assert_eq!(located("pages/api/types.d.ts"), None);
    }

    #[test]
    fn handler_methods_in_every_export_spelling() {
        let sink = extract(
            "app/api/x/route.ts",
            "export async function GET(request: Request) {}\n\
             export const POST = async (request: Request) => {};\n\
             const remove = () => {};\n\
             export { remove as DELETE };\n\
             export const PUT = remove;\n\
             export const { PATCH } = handlers;\n\
             export const dynamic = 'force-dynamic';\n",
        );
        assert_eq!(
            methods(&sink),
            vec!["GET", "POST", "DELETE", "PUT", "PATCH"]
        );
        assert!(sink.warnings.is_empty());
        assert_eq!(sink.routes[0].span.line, 1);
        assert_eq!(
            sink.routes[2].span.line, 4,
            "the export's line, not the function's"
        );
    }

    #[test]
    fn a_route_file_without_handlers_is_reported() {
        let sink = extract("app/api/x/route.ts", "export const runtime = 'edge';\n");
        assert!(sink.routes.is_empty());
        assert!(sink.warnings[0].contains("exports no HTTP method handler"));
    }

    #[test]
    fn every_route_file_attaches_to_one_root_per_app() {
        let sink = extract(
            "apps/web/src/app/api/x/route.ts",
            "export function GET() {}\n",
        );
        assert_eq!(sink.routers[0].symbol, SymbolId::new("apps/web", "next"));
        assert!(sink.routers[0].is_app_root);
        assert_eq!(sink.imports[0].source, "/apps/web");
        assert_eq!(sink.routes[0].router.name, "next");
    }

    #[test]
    fn handler_usage_becomes_params_and_body() {
        let sink = extract(
            "app/api/search/route.ts",
            "export async function POST(request: Request) {\n\
               const q = request.nextUrl.searchParams.get('q');\n\
               const { searchParams } = new URL(request.url);\n\
               const page = searchParams.get('page');\n\
               const key = request.headers.get('x-api-key');\n\
               const body = await request.json();\n\
             }\n",
        );
        let route = &sink.routes[0];
        let names: Vec<&str> = route.query_params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["q", "page"]);
        assert_eq!(route.headers[0].name, "x-api-key");
        assert_eq!(
            route.body.as_ref().unwrap().content_type,
            "application/json"
        );
    }

    #[test]
    fn pages_api_methods_are_read_from_the_checks() {
        let sink = extract(
            "pages/api/users.ts",
            "export default function handler(req, res) {\n\
               if (req.method === 'POST') { return; }\n\
               if (req.method !== 'DELETE') {}\n\
               switch (req.method) { case 'PUT': break; case 'GET': break; }\n\
               if (['PATCH'].includes(req.method)) {}\n\
             }\n",
        );
        assert_eq!(
            methods(&sink),
            vec!["POST", "DELETE", "PUT", "GET", "PATCH"]
        );
        assert!(sink.routes[0].summary.is_none());
    }

    #[test]
    fn a_pages_api_handler_that_never_checks_the_method_accepts_any() {
        let sink = extract(
            "pages/api/hello.ts",
            "export default (req, res) => { res.json({ q: req.query.name }); }\n",
        );
        assert_eq!(sink.routes[0].methods, UNSPECIFIED_METHODS.to_vec());
        assert!(sink.routes[0]
            .summary
            .as_deref()
            .unwrap()
            .contains("req.method"));
        assert_eq!(sink.routes[0].query_params[0].name, "name");
    }

    #[test]
    fn a_wrapped_pages_api_handler_is_followed_and_hints_at_auth() {
        let sink = extract(
            "pages/api/me.ts",
            "async function handler(req, res) { if (req.method === 'GET') {} }\n\
             export default withAuth(handler);\n",
        );
        assert_eq!(methods(&sink), vec!["GET"]);
        assert!(matches!(
            sink.routes[0].auth,
            Some(AuthRequirement::Unknown { ref hint }) if hint == "withAuth"
        ));
    }

    #[test]
    fn a_pages_api_file_without_a_default_export_is_reported() {
        let sink = extract("pages/api/util.ts", "export const helper = 1;\n");
        assert!(sink.routes.is_empty());
        assert!(sink.warnings[0].contains("no default export"));
    }

    #[test]
    fn routes_are_grouped_by_the_segment_after_api() {
        let sink = extract("app/api/users/[id]/route.ts", "export function GET() {}\n");
        assert_eq!(sink.routes[0].group.as_deref(), Some("users"));
        let sink = extract("app/health/route.ts", "export function GET() {}\n");
        assert_eq!(sink.routes[0].group.as_deref(), Some("health"));
    }
}
