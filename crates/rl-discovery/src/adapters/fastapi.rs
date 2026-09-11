//! FastAPI.
//!
//! Recognises `FastAPI()` and `APIRouter()` declarations, method decorators, `include_router`
//! mounts, and `add_api_route` calls — then hands them to the graph. It never joins a path.

use super::python::{
    auth_from_name, callee, collect_imports, docstring, enclosing_function, first_list_string,
    list_strings, Arguments, Constants,
};
use super::{Detection, FrameworkAdapter};
use crate::facts::{FactSink, MountFact, RouteFact, RouterFact, SymbolId, SymbolRef};
use crate::index::{walk, ParsedFile};
use crate::project::{Language, ProjectContext};
use rl_model::{AuthRequirement, BodySchema, HttpMethod, ParamSpec, TypeHint};
use std::path::PathBuf;
use tree_sitter::Node;

/// Decorator names that register a route.
const METHOD_DECORATORS: &[(&str, HttpMethod)] = &[
    ("get", HttpMethod::Get),
    ("post", HttpMethod::Post),
    ("put", HttpMethod::Put),
    ("patch", HttpMethod::Patch),
    ("delete", HttpMethod::Delete),
    ("head", HttpMethod::Head),
    ("options", HttpMethod::Options),
    ("trace", HttpMethod::Trace),
];

/// Parameters that are framework plumbing, not part of the API surface.
const PLUMBING_NAMES: &[&str] = &["self", "cls", "request", "response", "background_tasks"];
const PLUMBING_TYPES: &[&str] = &[
    "Request",
    "Response",
    "WebSocket",
    "BackgroundTasks",
    "UploadFile",
    "Session",
    "AsyncSession",
];

pub struct FastApiAdapter;

impl FrameworkAdapter for FastApiAdapter {
    fn id(&self) -> &'static str {
        "fastapi"
    }

    fn languages(&self) -> &[Language] {
        &[Language::Python]
    }

    fn detect(&self, project: &ProjectContext) -> Detection {
        let mut detection = Detection::none();

        if project.declares_dependency("fastapi") {
            detection.add(1, "`fastapi` is declared in a manifest");
        }

        // An actual import is much stronger evidence than a manifest entry: a package can sit
        // in a lockfile long after the code stopped using it.
        let mut imported_in = Vec::new();
        for path in project.files_of(Language::Python) {
            let Ok(source) = project.read(path) else {
                continue;
            };
            if source.contains("from fastapi import") || source.contains("import fastapi") {
                imported_in.push(path.clone());
                if imported_in.len() >= 3 {
                    break;
                }
            }
        }

        if !imported_in.is_empty() {
            detection.add(
                3,
                format!(
                    "fastapi is imported in {}",
                    crate::project::display(&imported_in[0])
                ),
            );
        }

        detection
    }

    fn candidate_files(&self, project: &ProjectContext) -> Vec<PathBuf> {
        project
            .files_of(Language::Python)
            .filter(|path| {
                // Tests and migrations declare no serving routes, and skipping them is the
                // main lever on scan speed in a large project.
                let text = path.to_string_lossy().replace('\\', "/");
                !text.contains("/tests/")
                    && !text.starts_with("tests/")
                    && !text.contains("/migrations/")
                    && !text
                        .rsplit('/')
                        .next()
                        .is_some_and(|f| f.starts_with("test_") || f.ends_with("_test.py"))
            })
            .cloned()
            .collect()
    }

    fn extract(&self, file: &ParsedFile, sink: &mut FactSink) {
        let constants = Constants::collect(file);

        for import in collect_imports(file) {
            sink.import(import);
        }

        walk(file.root(), &mut |node| match node.kind() {
            "assignment" => extract_declaration(file, node, &constants, sink),
            "call" => extract_call(file, node, &constants, sink),
            "decorated_definition" => extract_decorated(file, node, &constants, sink),
            _ => {}
        });

        if file.has_errors() {
            sink.warn(format!(
                "{} has syntax errors; routes around them may be missing",
                file.display_path()
            ));
        }
    }
}

/// `app = FastAPI(...)` or `router = APIRouter(prefix=..., tags=[...])`.
fn extract_declaration(
    file: &ParsedFile,
    node: Node<'_>,
    constants: &Constants,
    sink: &mut FactSink,
) {
    let (Some(left), Some(right)) = (
        node.child_by_field_name("left"),
        node.child_by_field_name("right"),
    ) else {
        return;
    };
    if left.kind() != "identifier" || right.kind() != "call" {
        return;
    }

    let Some((_, function)) = callee(file, right) else {
        return;
    };
    let is_app = function == "FastAPI";
    let is_router = function == "APIRouter";
    if !is_app && !is_router {
        return;
    }

    let args = Arguments::of(file, right);
    let prefix = args
        .keyword("prefix")
        .map(|n| constants.path_value(file, n))
        .unwrap_or_default();
    let group = args
        .keyword("tags")
        .and_then(|n| first_list_string(file, n, constants));

    sink.router(RouterFact {
        symbol: SymbolId::new(file.path.clone(), file.text(left)),
        prefix,
        group,
        is_app_root: is_app,
        factory: if is_app {
            enclosing_function(file, node)
        } else {
            None
        },
        span: file.span(left),
    });
}

/// `app.include_router(...)` and `router.add_api_route(...)`.
fn extract_call(file: &ParsedFile, node: Node<'_>, constants: &Constants, sink: &mut FactSink) {
    let Some((Some(object), method)) = callee(file, node) else {
        return;
    };
    let args = Arguments::of(file, node);

    match method.as_str() {
        "include_router" => {
            let Some(child) = args.first_positional() else {
                return;
            };
            let prefix = args
                .keyword("prefix")
                .map(|n| constants.path_value(file, n))
                .unwrap_or_default();
            let group = args
                .keyword("tags")
                .and_then(|n| first_list_string(file, n, constants));

            // `include_router(r, dependencies=[Depends(get_current_user)])` guards the
            // whole router.
            let auth = args
                .keyword("dependencies")
                .and_then(|list| auth_from_dependencies(file, list));

            sink.mount(MountFact {
                parent: SymbolRef::new(file.path.clone(), object),
                child: SymbolRef::new(file.path.clone(), file.text(child)),
                prefix,
                group,
                auth,
                methods: Vec::new(),
                replaces_child_prefix: false,
                span: file.span(node),
            });
        }

        "add_api_route" | "add_route" => {
            let Some(path_node) = args.first_positional() else {
                return;
            };
            let methods = args
                .keyword("methods")
                .map(|n| list_strings(file, n, constants))
                .unwrap_or_default()
                .iter()
                .filter_map(|m| m.parse::<HttpMethod>().ok())
                .collect::<Vec<_>>();

            let mut fact = RouteFact::new(
                SymbolRef::new(file.path.clone(), object),
                HttpMethod::Get,
                constants.path_value(file, path_node),
                file.span(node),
            );
            if !methods.is_empty() {
                fact.methods = methods;
            }
            fact.group = args
                .keyword("tags")
                .and_then(|n| first_list_string(file, n, constants));
            sink.route(fact);
        }

        _ => {}
    }
}

/// A decorated function — the common way a route is declared.
fn extract_decorated(
    file: &ParsedFile,
    node: Node<'_>,
    constants: &Constants,
    sink: &mut FactSink,
) {
    let Some(definition) = node.child_by_field_name("definition") else {
        return;
    };
    if definition.kind() != "function_definition" {
        return;
    }

    let mut cursor = node.walk();
    for decorator in node.children(&mut cursor) {
        if decorator.kind() != "decorator" {
            continue;
        }
        let Some(call) = decorator.named_child(0) else {
            continue;
        };
        if call.kind() != "call" {
            continue;
        }
        let Some((Some(object), method_name)) = callee(file, call) else {
            continue;
        };

        // `@router.get(...)`, or `@router.api_route(..., methods=[...])`.
        let methods = if let Some((_, method)) = METHOD_DECORATORS
            .iter()
            .find(|(name, _)| *name == method_name)
        {
            vec![method.clone()]
        } else if method_name == "api_route" || method_name == "route" {
            let args = Arguments::of(file, call);
            let listed = args
                .keyword("methods")
                .map(|n| list_strings(file, n, constants))
                .unwrap_or_default()
                .iter()
                .filter_map(|m| m.parse::<HttpMethod>().ok())
                .collect::<Vec<_>>();
            if listed.is_empty() {
                vec![HttpMethod::Get]
            } else {
                listed
            }
        } else {
            continue;
        };

        let args = Arguments::of(file, call);
        let path = args
            .first_positional()
            .map(|n| constants.path_value(file, n))
            .unwrap_or_default();

        let mut fact = RouteFact::new(
            SymbolRef::new(file.path.clone(), object),
            methods[0].clone(),
            path,
            file.span(decorator),
        );
        fact.methods = methods;

        fact.summary = args
            .keyword("summary")
            .and_then(|n| constants.string_value(file, n))
            .or_else(|| docstring(file, definition));
        fact.description = args
            .keyword("description")
            .and_then(|n| constants.string_value(file, n));
        fact.group = args
            .keyword("tags")
            .and_then(|n| first_list_string(file, n, constants));
        fact.deprecated = args
            .keyword("deprecated")
            .map(|n| file.text(n) == "True")
            .unwrap_or(false);

        // A route-level `dependencies=[Depends(...)]` is an auth signal in its own right.
        if let Some(dependencies) = args.keyword("dependencies") {
            fact.auth = auth_from_dependencies(file, dependencies);
        }

        let path_names: Vec<String> = fact
            .path
            .param_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        extract_signature(file, definition, &path_names, constants, &mut fact);

        sink.route(fact);
    }
}

/// Read the handler's parameters into query parameters, headers, a body, and an auth hint.
///
/// Best-effort by design. Where this cannot tell, it says nothing rather than inventing a
/// schema — [runtime enrich][enrich] is the answer when the schema really matters.
///
/// [enrich]: https://github.com/AryanMahajan/routelens/blob/main/docs/discovery/runtime-enrich.md
fn extract_signature(
    file: &ParsedFile,
    definition: Node<'_>,
    path_names: &[String],
    constants: &Constants,
    fact: &mut RouteFact,
) {
    let Some(parameters) = definition.child_by_field_name("parameters") else {
        return;
    };

    let mut cursor = parameters.walk();
    for parameter in parameters.named_children(&mut cursor) {
        let Some((name, type_node, default)) = parameter_parts(file, parameter) else {
            continue;
        };

        if PLUMBING_NAMES.contains(&name.as_str()) {
            continue;
        }
        let annotation = type_node
            .map(|n| file.text(n).to_string())
            .unwrap_or_default();
        if PLUMBING_TYPES.iter().any(|t| {
            annotation
                .split(['[', ']', '|', ' '])
                .any(|part| part == *t)
        }) {
            continue;
        }

        // Path parameters are already implied by the template.
        if path_names.contains(&name) {
            continue;
        }

        let marker = default.and_then(|d| {
            if d.kind() == "call" {
                callee(file, d).map(|(_, f)| (f, d))
            } else {
                None
            }
        });

        match marker.as_ref().map(|(f, node)| (f.as_str(), *node)) {
            Some(("Depends", node)) | Some(("Security", node)) => {
                if fact.auth.is_none() {
                    fact.auth = auth_from_dependency_call(file, node);
                }
            }
            Some(("Header", node)) => {
                fact.headers.push(build_param(
                    &header_name(&name),
                    type_node,
                    node,
                    file,
                    constants,
                ));
            }
            Some(("Query", node)) => {
                fact.query_params
                    .push(build_param(&name, type_node, node, file, constants));
            }
            Some(("Body", _)) | Some(("Form", _)) | Some(("File", _)) => {
                fact.body
                    .get_or_insert_with(|| json_body(&annotation, true));
            }
            Some(("Cookie", _)) => {}
            _ => {
                if looks_like_a_model(&annotation) {
                    fact.body
                        .get_or_insert_with(|| json_body(&annotation, default.is_none()));
                } else if !annotation.is_empty() || default.is_some() {
                    let mut spec = ParamSpec::new(&name);
                    spec.required = default.is_none() && !is_optional(&annotation);
                    spec.ty = type_hint(&annotation);
                    if let Some(default_node) = default {
                        spec.default = literal_json(file, default_node, constants);
                    }
                    fact.query_params.push(spec);
                }
            }
        }
    }
}

/// `(name, type, default)` for any of Python's parameter forms.
fn parameter_parts<'a>(
    file: &ParsedFile,
    parameter: Node<'a>,
) -> Option<(String, Option<Node<'a>>, Option<Node<'a>>)> {
    match parameter.kind() {
        "identifier" => Some((file.text(parameter).to_string(), None, None)),
        "typed_parameter" => {
            let name = parameter.named_child(0)?;
            Some((
                file.text(name).to_string(),
                parameter.child_by_field_name("type"),
                None,
            ))
        }
        "default_parameter" => {
            let name = parameter.child_by_field_name("name")?;
            Some((
                file.text(name).to_string(),
                None,
                parameter.child_by_field_name("value"),
            ))
        }
        "typed_default_parameter" => {
            let name = parameter.child_by_field_name("name")?;
            Some((
                file.text(name).to_string(),
                parameter.child_by_field_name("type"),
                parameter.child_by_field_name("value"),
            ))
        }
        _ => None,
    }
}

fn build_param(
    name: &str,
    type_node: Option<Node<'_>>,
    marker: Node<'_>,
    file: &ParsedFile,
    constants: &Constants,
) -> ParamSpec {
    let annotation = type_node
        .map(|n| file.text(n).to_string())
        .unwrap_or_default();
    let args = Arguments::of(file, marker);

    let mut spec = ParamSpec::new(name);
    spec.ty = type_hint(&annotation);
    spec.description = args
        .keyword("description")
        .and_then(|n| constants.string_value(file, n));

    // `Query(...)` — a literal ellipsis — is FastAPI's way of saying required.
    match args.first_positional() {
        Some(first) if file.text(first) == "..." => spec.required = true,
        Some(first) => {
            spec.required = false;
            spec.default = literal_json(file, first, constants);
        }
        None => spec.required = !is_optional(&annotation),
    }

    spec
}

fn json_body(annotation: &str, required: bool) -> BodySchema {
    // The model name is recorded rather than the model's fields: reconstructing a Pydantic
    // schema statically is exactly the job runtime enrich does properly.
    let title = annotation
        .split(['[', ']', '|', ' '])
        .find(|part| looks_like_a_model(part))
        .unwrap_or(annotation)
        .to_string();

    BodySchema {
        content_type: "application/json".to_string(),
        schema: (!title.is_empty())
            .then(|| serde_json::json!({ "type": "object", "title": title })),
        example: None,
        required,
    }
}

/// A capitalised annotation that is not a builtin is almost always a Pydantic model.
fn looks_like_a_model(annotation: &str) -> bool {
    let base = annotation
        .split(['[', ']', '|', ' ', ','])
        .map(str::trim)
        .find(|part| !part.is_empty() && *part != "Optional" && *part != "None")
        .unwrap_or("");

    const NOT_MODELS: &[&str] = &[
        "str", "int", "float", "bool", "bytes", "dict", "list", "set", "tuple", "Any", "Dict",
        "List", "Set", "Tuple", "UUID", "datetime", "date", "time", "Decimal",
    ];

    base.chars().next().is_some_and(char::is_uppercase) && !NOT_MODELS.contains(&base)
}

fn is_optional(annotation: &str) -> bool {
    annotation.contains("Optional") || annotation.contains("None")
}

fn type_hint(annotation: &str) -> Option<TypeHint> {
    let base = annotation
        .split(['[', ']', '|', ' ', ','])
        .map(str::trim)
        .find(|part| !part.is_empty() && *part != "Optional")?;

    Some(match base {
        "str" => TypeHint::String,
        "int" => TypeHint::Integer,
        "float" | "Decimal" => TypeHint::Number,
        "bool" => TypeHint::Boolean,
        "UUID" => TypeHint::Uuid,
        "date" => TypeHint::Date,
        "datetime" => TypeHint::DateTime,
        other => return Some(TypeHint::Other(other.to_string())),
    })
}

fn literal_json(
    file: &ParsedFile,
    node: Node<'_>,
    constants: &Constants,
) -> Option<serde_json::Value> {
    if let Some(text) = constants.string_value(file, node) {
        return Some(serde_json::Value::String(text));
    }
    let raw = file.text(node);
    match raw {
        "True" => Some(serde_json::Value::Bool(true)),
        "False" => Some(serde_json::Value::Bool(false)),
        "None" => Some(serde_json::Value::Null),
        _ => raw
            .parse::<i64>()
            .ok()
            .map(serde_json::Value::from)
            .or_else(|| {
                raw.parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(serde_json::Value::Number)
            }),
    }
}

/// `user_id` → `X-User-Id`, which is FastAPI's own convention for `Header` parameters.
fn header_name(name: &str) -> String {
    name.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

fn auth_from_dependencies(file: &ParsedFile, list: Node<'_>) -> Option<AuthRequirement> {
    (0..list.named_child_count() as u32)
        .filter_map(|i| list.named_child(i))
        .find_map(|item| auth_from_dependency_call(file, item))
}

/// Turn `Depends(get_current_user)` into an auth requirement.
///
/// The scheme is guessed from the dependency's name, which is a heuristic — so anything
/// unrecognised becomes [`AuthRequirement::Unknown`] carrying the name, rather than a
/// confident claim.
fn auth_from_dependency_call(file: &ParsedFile, node: Node<'_>) -> Option<AuthRequirement> {
    if node.kind() != "call" {
        return None;
    }
    let (_, function) = callee(file, node)?;
    if function != "Depends" && function != "Security" {
        return None;
    }

    let args = Arguments::of(file, node);
    let hint = args
        .first_positional()
        .map(|n| file.text(n).to_string())
        .unwrap_or_else(|| function.clone());
    auth_from_name(&hint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::RegistrationGraph;
    use crate::index::SourceIndex;
    use crate::project::Language;
    use rl_model::ParamStyle;
    use std::collections::BTreeSet;

    /// Parse several modules and resolve them, as a real scan would.
    fn discover(files: &[(&str, &str)]) -> (Vec<String>, Vec<String>) {
        let mut index = SourceIndex::new().unwrap();
        let mut sink = FactSink::new();
        let adapter = FastApiAdapter;

        for (path, source) in files {
            let parsed = index
                .parse(*path, Language::Python, source.to_string())
                .unwrap();
            adapter.extract(&parsed, &mut sink);
        }

        let known: BTreeSet<PathBuf> = files.iter().map(|(p, _)| PathBuf::from(p)).collect();
        let graph = RegistrationGraph::build(sink, &known);
        let resolution = graph.resolve();

        let mut routes: Vec<String> = resolution
            .routes
            .iter()
            .map(|r| format!("{} {}", r.method, r.path.render(ParamStyle::Braces)))
            .collect();
        routes.sort();

        let warnings = resolution.warnings.iter().map(|w| w.to_string()).collect();
        (routes, warnings)
    }

    fn single(source: &str) -> Vec<String> {
        discover(&[("main.py", source)]).0
    }

    /// Parse one file and return the first route's facts, for signature assertions.
    fn one_route(source: &str) -> RouteFact {
        let mut index = SourceIndex::new().unwrap();
        let mut sink = FactSink::new();
        let parsed = index
            .parse("main.py", Language::Python, source.to_string())
            .unwrap();
        FastApiAdapter.extract(&parsed, &mut sink);
        sink.routes.into_iter().next().expect("no route found")
    }

    #[test]
    fn include_router_dependencies_guard_the_mount() {
        let mut index = SourceIndex::new().unwrap();
        let mut sink = FactSink::new();
        let parsed = index
            .parse(
                "main.py",
                Language::Python,
                "app.include_router(admin.router, prefix=\"/admin\", dependencies=[Depends(get_current_admin)])
                 app.include_router(public.router, prefix=\"/public\")
"
                    .to_string(),
            )
            .unwrap();
        FastApiAdapter.extract(&parsed, &mut sink);

        assert_eq!(sink.mounts.len(), 2);
        assert!(matches!(
            sink.mounts[0].auth,
            Some(AuthRequirement::Unknown { ref hint }) if hint.contains("get_current_admin")
        ));
        assert!(sink.mounts[1].auth.is_none());
    }

    /// Running the fixture found this: FastAPI redirects `/api/v1/users` to
    /// `/api/v1/users/` for a router declared with `@router.get("/")`, so the discovered
    /// path must carry the slash — while `@app.get("/")` on the root is still just `/`.
    #[test]
    fn a_bare_slash_route_keeps_its_slash_under_a_prefix() {
        let (routes, _) = discover(&[
            (
                "api/users.py",
                "from fastapi import APIRouter
                 router = APIRouter(prefix=\"/users\")
                 @router.get(\"/\")
                 def list_users(): ...
",
            ),
            (
                "main.py",
                "from fastapi import FastAPI
                 from api.users import router
                 app = FastAPI()
                 app.include_router(router, prefix=\"/api/v1\")
                 @app.get(\"/\")
                 def root(): ...
",
            ),
        ]);
        assert_eq!(routes, vec!["GET /", "GET /api/v1/users/"]);
    }

    #[test]
    fn finds_a_route_declared_straight_on_the_app() {
        let routes = single(
            "from fastapi import FastAPI\n\
             app = FastAPI()\n\
             \n\
             @app.get(\"/health\")\n\
             async def health():\n    return {}\n",
        );
        assert_eq!(routes, vec!["GET /health"]);
    }

    #[test]
    fn every_method_decorator_is_recognised() {
        let routes = single(
            "from fastapi import FastAPI\n\
             app = FastAPI()\n\
             @app.get(\"/a\")\n\
             def a(): ...\n\
             @app.post(\"/a\")\n\
             def b(): ...\n\
             @app.delete(\"/a\")\n\
             def c(): ...\n\
             @app.patch(\"/a\")\n\
             def d(): ...\n",
        );
        assert_eq!(routes, vec!["DELETE /a", "GET /a", "PATCH /a", "POST /a"]);
    }

    /// The shape from `docs/discovery/how-it-works.md`.
    #[test]
    fn composes_a_router_prefix_with_its_mount_prefix_across_files() {
        let (routes, _) = discover(&[
            (
                "api/users.py",
                "from fastapi import APIRouter\n\
                 router = APIRouter(prefix=\"/users\", tags=[\"users\"])\n\
                 \n\
                 @router.get(\"/{user_id}\")\n\
                 async def get_user(user_id: int): ...\n",
            ),
            (
                "main.py",
                "from fastapi import FastAPI\n\
                 from api.users import router\n\
                 app = FastAPI()\n\
                 app.include_router(router, prefix=\"/api/v1\")\n",
            ),
        ]);

        assert_eq!(routes, vec!["GET /api/v1/users/{user_id}"]);
    }

    #[test]
    fn a_relative_import_links_the_files() {
        let (routes, _) = discover(&[
            (
                "app/api/items.py",
                "from fastapi import APIRouter\n\
                 router = APIRouter(prefix=\"/items\")\n\
                 @router.get(\"/\")\n\
                 def list_items(): ...\n",
            ),
            (
                "app/main.py",
                "from fastapi import FastAPI\n\
                 from .api.items import router\n\
                 app = FastAPI()\n\
                 app.include_router(router)\n",
            ),
        ]);
        assert_eq!(routes, vec!["GET /items/"]);
    }

    #[test]
    fn a_prefix_held_in_a_constant_is_folded() {
        let routes = single(
            "from fastapi import FastAPI, APIRouter\n\
             API_PREFIX = \"/api/v1\"\n\
             app = FastAPI()\n\
             router = APIRouter()\n\
             @router.get(\"/ping\")\n\
             def ping(): ...\n\
             app.include_router(router, prefix=API_PREFIX)\n",
        );
        assert_eq!(routes, vec!["GET /api/v1/ping"]);
    }

    /// The honesty guarantee: a prefix that cannot be folded is shown as a gap.
    #[test]
    fn an_unresolvable_prefix_becomes_a_visible_gap_not_a_guess() {
        let routes = single(
            "from fastapi import FastAPI, APIRouter\n\
             from .config import settings\n\
             app = FastAPI()\n\
             router = APIRouter()\n\
             @router.get(\"/ping\")\n\
             def ping(): ...\n\
             app.include_router(router, prefix=settings.API_PREFIX)\n",
        );
        assert_eq!(routes, vec!["GET /?/ping"]);
    }

    #[test]
    fn a_router_mounted_twice_yields_both_paths() {
        let routes = single(
            "from fastapi import FastAPI, APIRouter\n\
             app = FastAPI()\n\
             router = APIRouter()\n\
             @router.get(\"/items\")\n\
             def items(): ...\n\
             app.include_router(router, prefix=\"/v1\")\n\
             app.include_router(router, prefix=\"/v2\")\n",
        );
        assert_eq!(routes, vec!["GET /v1/items", "GET /v2/items"]);
    }

    #[test]
    fn an_unmounted_router_is_reported_but_its_routes_are_kept() {
        let (routes, warnings) = discover(&[(
            "main.py",
            "from fastapi import FastAPI, APIRouter\n\
             app = FastAPI()\n\
             orphan = APIRouter(prefix=\"/orphan\")\n\
             @orphan.get(\"/x\")\n\
             def x(): ...\n",
        )]);

        assert_eq!(routes, vec!["GET /orphan/x"]);
        assert!(warnings.iter().any(|w| w.contains("never mounted")));
    }

    #[test]
    fn api_route_with_a_method_list_becomes_several_routes() {
        let routes = single(
            "from fastapi import FastAPI\n\
             app = FastAPI()\n\
             @app.api_route(\"/thing\", methods=[\"GET\", \"POST\"])\n\
             def thing(): ...\n",
        );
        assert_eq!(routes, vec!["GET /thing", "POST /thing"]);
    }

    #[test]
    fn add_api_route_is_recognised() {
        let routes = single(
            "from fastapi import FastAPI\n\
             app = FastAPI()\n\
             app.add_api_route(\"/legacy\", handler, methods=[\"PUT\"])\n",
        );
        assert_eq!(routes, vec!["PUT /legacy"]);
    }

    #[test]
    fn tags_become_the_group() {
        let mut index = SourceIndex::new().unwrap();
        let mut sink = FactSink::new();
        let parsed = index
            .parse(
                "main.py",
                Language::Python,
                "from fastapi import APIRouter\n\
                 router = APIRouter(tags=[\"users\"])\n\
                 @router.get(\"/x\")\n\
                 def x(): ...\n"
                    .to_string(),
            )
            .unwrap();
        FastApiAdapter.extract(&parsed, &mut sink);

        assert_eq!(sink.routers[0].group.as_deref(), Some("users"));
    }

    #[test]
    fn query_parameters_are_read_from_the_signature() {
        let route = one_route(
            "from fastapi import FastAPI\n\
             app = FastAPI()\n\
             @app.get(\"/search\")\n\
             def search(q: str, limit: int = 10): ...\n",
        );

        let names: Vec<&str> = route.query_params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["q", "limit"]);

        assert!(route.query_params[0].required, "no default means required");
        assert_eq!(route.query_params[0].ty, Some(TypeHint::String));

        assert!(!route.query_params[1].required);
        assert_eq!(route.query_params[1].default, Some(serde_json::json!(10)));
    }

    #[test]
    fn a_path_parameter_is_not_repeated_as_a_query_parameter() {
        let route = one_route(
            "from fastapi import FastAPI\n\
             app = FastAPI()\n\
             @app.get(\"/users/{user_id}\")\n\
             def get_user(user_id: int, verbose: bool = False): ...\n",
        );

        let names: Vec<&str> = route.query_params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["verbose"]);
    }

    #[test]
    fn a_query_marker_declares_requiredness_explicitly() {
        let route = one_route(
            "from fastapi import FastAPI, Query\n\
             app = FastAPI()\n\
             @app.get(\"/search\")\n\
             def search(q: str = Query(...), page: int = Query(1)): ...\n",
        );

        assert!(route.query_params[0].required);
        assert!(!route.query_params[1].required);
        assert_eq!(route.query_params[1].default, Some(serde_json::json!(1)));
    }

    #[test]
    fn a_pydantic_model_parameter_becomes_the_request_body() {
        let route = one_route(
            "from fastapi import FastAPI\n\
             app = FastAPI()\n\
             @app.post(\"/users\")\n\
             def create(payload: UserCreate): ...\n",
        );

        let body = route.body.expect("expected a request body");
        assert_eq!(body.content_type, "application/json");
        assert!(body.required);
        assert_eq!(
            body.schema.unwrap().get("title").and_then(|t| t.as_str()),
            Some("UserCreate")
        );
        assert!(route.query_params.is_empty());
    }

    #[test]
    fn framework_plumbing_parameters_are_ignored() {
        let route = one_route(
            "from fastapi import FastAPI\n\
             app = FastAPI()\n\
             @app.get(\"/x\")\n\
             def x(request: Request, db: Session = Depends(get_db)): ...\n",
        );

        assert!(route.query_params.is_empty());
        assert!(route.body.is_none());
        assert!(route.auth.is_none(), "a database session is not auth");
    }

    #[test]
    fn a_security_dependency_becomes_an_auth_requirement() {
        let bearer = one_route(
            "from fastapi import FastAPI\n\
             app = FastAPI()\n\
             @app.get(\"/me\")\n\
             def me(user = Depends(oauth2_scheme)): ...\n",
        );
        assert_eq!(bearer.auth, Some(AuthRequirement::Bearer { format: None }));

        let vague = one_route(
            "from fastapi import FastAPI\n\
             app = FastAPI()\n\
             @app.get(\"/me\")\n\
             def me(user = Depends(get_current_user)): ...\n",
        );
        assert_eq!(
            vague.auth,
            Some(AuthRequirement::Unknown {
                hint: "get_current_user".into()
            })
        );
    }

    #[test]
    fn a_header_parameter_is_named_the_way_the_header_is_spelled() {
        let route = one_route(
            "from fastapi import FastAPI, Header\n\
             app = FastAPI()\n\
             @app.get(\"/x\")\n\
             def x(user_agent: str = Header(None)): ...\n",
        );
        assert_eq!(route.headers[0].name, "User-Agent");
    }

    #[test]
    fn a_docstring_becomes_the_summary_when_the_decorator_gives_none() {
        let route = one_route(
            "from fastapi import FastAPI\n\
             app = FastAPI()\n\
             @app.get(\"/x\")\n\
             def x():\n    \"\"\"List all the things.\"\"\"\n    return []\n",
        );
        assert_eq!(route.summary.as_deref(), Some("List all the things."));
    }

    #[test]
    fn an_explicit_summary_wins_over_the_docstring() {
        let route = one_route(
            "from fastapi import FastAPI\n\
             app = FastAPI()\n\
             @app.get(\"/x\", summary=\"Explicit\")\n\
             def x():\n    \"\"\"Docstring.\"\"\"\n    return []\n",
        );
        assert_eq!(route.summary.as_deref(), Some("Explicit"));
    }

    #[test]
    fn a_file_with_syntax_errors_still_yields_the_routes_around_it() {
        let mut index = SourceIndex::new().unwrap();
        let mut sink = FactSink::new();
        let parsed = index
            .parse(
                "main.py",
                Language::Python,
                "from fastapi import FastAPI\n\
                 app = FastAPI()\n\
                 @app.get(\"/ok\")\n\
                 def ok(): ...\n\
                 def broken(:\n    pass\n"
                    .to_string(),
            )
            .unwrap();
        FastApiAdapter.extract(&parsed, &mut sink);

        assert_eq!(sink.routes.len(), 1);
        assert!(sink.warnings.iter().any(|w| w.contains("syntax errors")));
    }

    #[test]
    fn detection_weighs_an_import_more_heavily_than_a_manifest_entry() {
        use crate::project::ProjectContext;
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("requirements.txt"), "fastapi==0.110\n").unwrap();
        let manifest_only = ProjectContext::scan(dir.path()).unwrap();
        let weak = FastApiAdapter.detect(&manifest_only);

        fs::write(
            dir.path().join("main.py"),
            "from fastapi import FastAPI\napp = FastAPI()\n",
        )
        .unwrap();
        let with_import = ProjectContext::scan(dir.path()).unwrap();
        let strong = FastApiAdapter.detect(&with_import);

        assert!(weak.matched());
        assert!(strong.score > weak.score);
    }

    #[test]
    fn test_files_are_not_scanned() {
        use crate::project::ProjectContext;
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("tests")).unwrap();
        fs::write(dir.path().join("main.py"), "").unwrap();
        fs::write(dir.path().join("tests/test_api.py"), "").unwrap();

        let project = ProjectContext::scan(dir.path()).unwrap();
        let candidates = FastApiAdapter.candidate_files(&project);

        assert!(candidates.iter().any(|p| p.ends_with("main.py")));
        assert!(!candidates
            .iter()
            .any(|p| p.to_string_lossy().contains("test_api")));
    }
}
