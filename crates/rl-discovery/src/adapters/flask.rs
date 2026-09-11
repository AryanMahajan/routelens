//! Flask.
//!
//! Recognises `Flask()` and `Blueprint()` declarations, `@app.route` / `@bp.get` decorators,
//! `add_url_rule`, `register_blueprint` mounts, and class-based views (`MethodView`,
//! Flask-RESTful `Resource`, Flask-RESTX `Namespace`). It never joins a path: a blueprint's
//! `url_prefix` and the `register_blueprint(url_prefix=)` on top of it are composed by the
//! graph, exactly as FastAPI's `include_router` is.
//!
//! A class-based view is modelled as a *router* whose routes have empty paths — one per
//! `get`/`post`/… method — and `add_url_rule("/notes/<int:id>", view_func=NoteAPI.as_view())`
//! as a *mount* of that router at the rule. The graph then links the class in `views.py` to
//! its registration in `__init__.py` through the import, the same way it links a blueprint.
//!
//! What a handler *reads* is the only source of query, header and body information — Flask
//! has no signature-level declaration of them — so `request.args.get("q")` is read out of
//! the body. Best-effort by design; runtime enrich's `url_map` walk is the exact answer for
//! paths and methods, though Flask has no schemas to offer even at runtime.

use super::python::{
    auth_from_name, callee, collect_imports, docstring, enclosing_function, list_strings,
    Arguments, Constants, RequestDialect, RequestUsage,
};
use super::{Detection, FrameworkAdapter};
use crate::facts::{FactSink, MountFact, RouteFact, RouterFact, SymbolId, SymbolRef};
use crate::index::{walk, ParsedFile};
use crate::project::{Language, ProjectContext};
use rl_model::{AuthRequirement, HttpMethod, ParamStyle, PathTemplate};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use tree_sitter::Node;

/// Decorator and call names that register a route with one method (Flask ≥ 2.0).
const METHOD_SHORTCUTS: &[(&str, HttpMethod)] = &[
    ("get", HttpMethod::Get),
    ("post", HttpMethod::Post),
    ("put", HttpMethod::Put),
    ("patch", HttpMethod::Patch),
    ("delete", HttpMethod::Delete),
];

/// Method names on a `MethodView` / `Resource` class that serve a request.
const VIEW_METHODS: &[(&str, HttpMethod)] = &[
    ("get", HttpMethod::Get),
    ("post", HttpMethod::Post),
    ("put", HttpMethod::Put),
    ("patch", HttpMethod::Patch),
    ("delete", HttpMethod::Delete),
    ("head", HttpMethod::Head),
    ("options", HttpMethod::Options),
];

pub struct FlaskAdapter;

impl FrameworkAdapter for FlaskAdapter {
    fn id(&self) -> &'static str {
        "flask"
    }

    fn languages(&self) -> &[Language] {
        &[Language::Python]
    }

    fn detect(&self, project: &ProjectContext) -> Detection {
        let mut detection = Detection::none();

        if project.declares_dependency("flask") {
            detection.add(1, "`flask` is declared in a manifest");
        }

        let mut imported_in = Vec::new();
        for path in project.files_of(Language::Python) {
            let Ok(source) = project.read(path) else {
                continue;
            };
            if source.contains("from flask import") || source.contains("import flask") {
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
                    "flask is imported in {}",
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

        let classes = emit_view_classes(file, sink);
        let mut extractor = Extractor {
            file,
            constants: &constants,
            classes,
            view_aliases: view_aliases(file),
            sink,
        };

        walk(file.root(), &mut |node| match node.kind() {
            "assignment" => extractor.declaration(node),
            "call" => extractor.call(node),
            "decorated_definition" => extractor.decorated(node),
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

struct Extractor<'a> {
    file: &'a ParsedFile,
    constants: &'a Constants,
    /// Class-based views declared in this file.
    classes: BTreeSet<String>,
    /// `note_view = NoteAPI.as_view("note")` — the local name a class-based view travels under.
    view_aliases: BTreeMap<String, String>,
    sink: &'a mut FactSink,
}

impl Extractor<'_> {
    fn path(&self, node: Node<'_>) -> PathTemplate {
        self.constants
            .path_value_in(self.file, node, ParamStyle::Angle)
    }

    fn text(&self, node: Node<'_>) -> &str {
        self.file.text(node)
    }

    fn reference(&self, name: &str) -> SymbolRef {
        SymbolRef::new(self.file.path.clone(), name)
    }

    /// `app = Flask(__name__)`, `bp = Blueprint("users", __name__, url_prefix="/users")`,
    /// `api = Api(app, prefix="/api")`, `ns = Namespace("users", path="/users")`.
    fn declaration(&mut self, node: Node<'_>) {
        let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        ) else {
            return;
        };
        if left.kind() != "identifier" || right.kind() != "call" {
            return;
        }
        let Some((_, function)) = callee(self.file, right) else {
            return;
        };
        let args = Arguments::of(self.file, right);

        let (prefix, group, is_app_root) = match function.as_str() {
            "Flask" => (PathTemplate::empty(), None, true),
            "Blueprint" => (
                self.keyword_path(&args, "url_prefix"),
                args.first_positional()
                    .and_then(|n| self.constants.string_value(self.file, n)),
                false,
            ),
            // Flask-RESTX: `Namespace("users", description=..., path="/users")`.
            "Namespace" => (
                self.keyword_path(&args, "path"),
                args.first_positional()
                    .and_then(|n| self.constants.string_value(self.file, n)),
                false,
            ),
            // Flask-RESTful / RESTX: `Api(app, prefix="/api")` both declares and mounts.
            "Api" => {
                let prefix = self.keyword_path(&args, "prefix");
                if let Some(app) = args.first_positional() {
                    if app.kind() == "identifier" {
                        self.sink.mount(MountFact {
                            parent: self.reference(self.text(app)),
                            child: self.reference(self.text(left)),
                            prefix: PathTemplate::empty(),
                            group: None,
                            auth: None,
                            methods: Vec::new(),
                            replaces_child_prefix: false,
                            span: self.file.span(right),
                        });
                    }
                }
                (prefix, None, false)
            }
            _ => return,
        };

        self.sink.router(RouterFact {
            symbol: SymbolId::new(self.file.path.clone(), self.text(left)),
            prefix,
            group,
            is_app_root,
            factory: if is_app_root {
                enclosing_function(self.file, node)
            } else {
                None
            },
            implicit: false,
            span: self.file.span(left),
        });
    }

    fn keyword_path(&self, args: &Arguments<'_>, name: &str) -> PathTemplate {
        args.keyword(name).map(|n| self.path(n)).unwrap_or_default()
    }

    /// `app.register_blueprint(bp, url_prefix="/api")`, `api.add_namespace(ns, path=)`,
    /// `api.init_app(app)`, `app.add_url_rule(...)`, `api.add_resource(...)`.
    fn call(&mut self, node: Node<'_>) {
        let Some((Some(object), method)) = callee(self.file, node) else {
            return;
        };
        let args = Arguments::of(self.file, node);

        match method.as_str() {
            "register_blueprint" | "add_namespace" => {
                let Some(child) = args.first_positional() else {
                    return;
                };
                let key = if method == "add_namespace" {
                    "path"
                } else {
                    "url_prefix"
                };
                // `register_blueprint(bp, url_prefix="/x")` *replaces* the blueprint's own
                // `url_prefix`; only when none is given does the blueprint's apply. The
                // same holds for a RESTX namespace's `path`.
                let explicit = args.keyword(key).is_some();
                self.sink.mount(MountFact {
                    parent: self.reference(&object),
                    child: self.reference(self.text(child)),
                    prefix: self.keyword_path(&args, key),
                    group: None,
                    auth: None,
                    methods: Vec::new(),
                    replaces_child_prefix: explicit,
                    span: self.file.span(node),
                });
            }

            // `api.init_app(app)` — the deferred form of `Api(app)`.
            "init_app" => {
                let Some(app) = args.first_positional() else {
                    return;
                };
                if app.kind() != "identifier" {
                    return;
                }
                self.sink.mount(MountFact {
                    parent: self.reference(self.text(app)),
                    child: self.reference(&object),
                    prefix: PathTemplate::empty(),
                    group: None,
                    auth: None,
                    methods: Vec::new(),
                    replaces_child_prefix: false,
                    span: self.file.span(node),
                });
            }

            "add_url_rule" => {
                let Some(rule) = args.first_positional() else {
                    return;
                };
                let view = args
                    .keyword("view_func")
                    .or_else(|| args.positional.get(2).copied());
                let methods = args
                    .keyword("methods")
                    .map(|n| self.methods_of(n))
                    .unwrap_or_default();
                self.register_view(&object, rule, view, methods, node);
            }

            // Flask-RESTful: `api.add_resource(TodoList, "/todos", "/todos/")`.
            "add_resource" => {
                let Some(class) = args.first_positional() else {
                    return;
                };
                for rule in args.positional.iter().skip(1) {
                    self.register_view(&object, *rule, Some(class), Vec::new(), node);
                }
            }

            _ => {}
        }
    }

    /// A route registered by a call rather than a decorator. `view` may be a function, a
    /// class, or `Class.as_view("name")`.
    fn register_view(
        &mut self,
        router: &str,
        rule: Node<'_>,
        view: Option<Node<'_>>,
        explicit_methods: Vec<HttpMethod>,
        call: Node<'_>,
    ) {
        let path = self.path(rule);
        let span = self.file.span(call);

        match view.map(|v| self.view_target(v)) {
            Some(ViewTarget::Class(class)) => {
                self.sink.mount(MountFact {
                    parent: self.reference(router),
                    child: self.reference(&class),
                    prefix: path,
                    group: None,
                    auth: None,
                    methods: explicit_methods,
                    replaces_child_prefix: false,
                    span,
                });
            }
            Some(ViewTarget::Function(name)) => {
                let mut fact = RouteFact::new(self.reference(router), HttpMethod::Get, path, span);
                if !explicit_methods.is_empty() {
                    fact.methods = explicit_methods;
                }
                if let Some(function) = self.top_level_function(&name) {
                    self.apply_handler(function, &mut fact);
                }
                self.sink.route(fact);
            }
            Some(ViewTarget::Unknown) | None => {
                // `view_func=lambda: ...` or an expression: the rule is real even if nothing
                // more can be said about it.
                let mut fact = RouteFact::new(self.reference(router), HttpMethod::Get, path, span);
                if !explicit_methods.is_empty() {
                    fact.methods = explicit_methods;
                }
                self.sink.route(fact);
            }
        }
    }

    /// What a `view_func=` argument names.
    ///
    /// A name declared in this file is known exactly; an imported one is a class when it is
    /// spelled like one (`NoteAPI`), which is PEP 8 and a heuristic.
    fn view_target(&self, view: Node<'_>) -> ViewTarget {
        match view.kind() {
            "identifier" => {
                let name = self.text(view);
                if let Some(class) = self.view_aliases.get(name) {
                    return ViewTarget::Class(class.clone());
                }
                if self.classes.contains(name) {
                    return ViewTarget::Class(name.to_string());
                }
                if self.top_level_function(name).is_some() {
                    return ViewTarget::Function(name.to_string());
                }
                if name.chars().next().is_some_and(char::is_uppercase) {
                    ViewTarget::Class(name.to_string())
                } else {
                    ViewTarget::Function(name.to_string())
                }
            }
            "call" => match callee(self.file, view) {
                Some((Some(class), function)) if function == "as_view" => ViewTarget::Class(class),
                _ => ViewTarget::Unknown,
            },
            "attribute" => ViewTarget::Function(self.text(view).to_string()),
            _ => ViewTarget::Unknown,
        }
    }

    /// `@app.route("/x", methods=[...])`, `@bp.get("/x")`, and `@ns.route("/x")` on a class.
    fn decorated(&mut self, node: Node<'_>) {
        let Some(definition) = node.child_by_field_name("definition") else {
            return;
        };

        let mut route_decorators = Vec::new();
        let mut auth = None;
        let mut cursor = node.walk();
        for decorator in node.children(&mut cursor) {
            if decorator.kind() != "decorator" {
                continue;
            }
            let Some(expression) = decorator.named_child(0) else {
                continue;
            };

            let routed = expression.kind() == "call"
                && callee(self.file, expression).is_some_and(|(object, method)| {
                    object.is_some()
                        && (method == "route"
                            || METHOD_SHORTCUTS.iter().any(|(name, _)| *name == method))
                });
            if routed {
                route_decorators.push((decorator, expression));
            } else if auth.is_none() {
                // `@login_required`, `@jwt_required()`, `@auth.login_required`.
                auth = self.auth_from_decorator(expression);
            }
        }

        for (decorator, call) in route_decorators {
            let Some((Some(object), method_name)) = callee(self.file, call) else {
                continue;
            };
            let args = Arguments::of(self.file, call);
            let path = args
                .first_positional()
                .map(|n| self.path(n))
                .unwrap_or_default();
            let span = self.file.span(decorator);

            if definition.kind() == "class_definition" {
                // Flask-RESTX: `@ns.route("/<int:id>")` over a `Resource` class.
                let Some(name) = definition.child_by_field_name("name") else {
                    continue;
                };
                self.sink.mount(MountFact {
                    parent: self.reference(&object),
                    child: self.reference(self.text(name)),
                    prefix: path,
                    group: None,
                    auth: None,
                    methods: Vec::new(),
                    replaces_child_prefix: false,
                    span,
                });
                continue;
            }

            let methods = match METHOD_SHORTCUTS
                .iter()
                .find(|(name, _)| *name == method_name)
            {
                Some((_, method)) => vec![method.clone()],
                None => {
                    let listed = args
                        .keyword("methods")
                        .map(|n| self.methods_of(n))
                        .unwrap_or_default();
                    if listed.is_empty() {
                        vec![HttpMethod::Get]
                    } else {
                        listed
                    }
                }
            };

            let mut fact = RouteFact::new(self.reference(&object), methods[0].clone(), path, span);
            fact.methods = methods;
            fact.auth = auth.clone();
            self.apply_handler(definition, &mut fact);
            self.sink.route(fact);
        }
    }

    fn apply_handler(&self, definition: Node<'_>, fact: &mut RouteFact) {
        let path_names = fact.path.param_names();
        let usage = RequestUsage::of(self.file, definition, &path_names, &REQUEST);
        usage.apply(fact);
        if fact.summary.is_none() {
            fact.summary = docstring(self.file, definition);
        }
    }

    fn methods_of(&self, node: Node<'_>) -> Vec<HttpMethod> {
        list_strings(self.file, node, self.constants)
            .iter()
            .filter_map(|m| m.parse::<HttpMethod>().ok())
            .collect()
    }

    fn auth_from_decorator(&self, expression: Node<'_>) -> Option<AuthRequirement> {
        let name = match expression.kind() {
            "call" => {
                let (object, method) = callee(self.file, expression)?;
                match object {
                    Some(object) => format!("{object}.{method}"),
                    None => method,
                }
            }
            "identifier" | "attribute" => self.text(expression).to_string(),
            _ => return None,
        };
        auth_from_decorator_name(&name)
    }

    fn top_level_function<'a>(&'a self, name: &str) -> Option<Node<'a>> {
        let root = self.file.root();
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            let definition = match child.kind() {
                "function_definition" => child,
                "decorated_definition" => child.child_by_field_name("definition")?,
                _ => continue,
            };
            if definition.kind() == "function_definition"
                && definition
                    .child_by_field_name("name")
                    .is_some_and(|n| self.text(n) == name)
            {
                return Some(definition);
            }
        }
        None
    }
}

/// Local names bound to `Class.as_view(...)` calls, anywhere in the file.
fn view_aliases(file: &ParsedFile) -> BTreeMap<String, String> {
    let mut aliases = BTreeMap::new();
    walk(file.root(), &mut |node| {
        if node.kind() != "assignment" {
            return;
        }
        let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        ) else {
            return;
        };
        if left.kind() != "identifier" || right.kind() != "call" {
            return;
        }
        if let Some((Some(class), function)) = callee(file, right) {
            if function == "as_view" {
                aliases.insert(file.text(left).to_string(), class);
            }
        }
    });
    aliases
}

/// `@login_required` is Flask-Login's session cookie; `@jwt_required()` is a bearer token.
/// Everything else goes through the shared name heuristic.
fn auth_from_decorator_name(name: &str) -> Option<AuthRequirement> {
    let lowered = name.to_ascii_lowercase();
    if lowered == "login_required" || lowered.ends_with(".login_required") {
        let via_http_auth = lowered.contains("basic");
        return Some(if via_http_auth {
            AuthRequirement::Basic
        } else if lowered.contains("token") || lowered.contains("jwt") {
            AuthRequirement::Bearer { format: None }
        } else if lowered == "login_required" {
            AuthRequirement::Cookie {
                name: "session".to_string(),
            }
        } else {
            AuthRequirement::Unknown {
                hint: name.to_string(),
            }
        });
    }
    auth_from_name(name)
}

enum ViewTarget {
    Class(String),
    Function(String),
    Unknown,
}

/// Emit every class-based view in the file as a router with one empty-path route per
/// implemented HTTP method, and return their names.
///
/// Only classes with a `get`/`post`/… method count, so a plain data class is never mistaken
/// for a view. A `methods = [...]` attribute narrows the list; a `decorators = [...]`
/// attribute or a decorator on the class guards every method.
fn emit_view_classes(file: &ParsedFile, sink: &mut FactSink) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let root = file.root();
    let mut cursor = root.walk();

    for child in root.named_children(&mut cursor) {
        let (definition, decorated) = match child.kind() {
            "class_definition" => (child, None),
            "decorated_definition" => match child.child_by_field_name("definition") {
                Some(d) if d.kind() == "class_definition" => (d, Some(child)),
                _ => continue,
            },
            _ => continue,
        };
        let (Some(name), Some(body)) = (
            definition.child_by_field_name("name"),
            definition.child_by_field_name("body"),
        ) else {
            continue;
        };

        let mut methods: Vec<(HttpMethod, Node<'_>)> = Vec::new();
        let mut auth = None;
        let mut declared_methods: Option<Vec<HttpMethod>> = None;
        let mut inner = body.walk();
        for statement in body.named_children(&mut inner) {
            let function = match statement.kind() {
                "function_definition" => Some(statement),
                "decorated_definition" => statement
                    .child_by_field_name("definition")
                    .filter(|d| d.kind() == "function_definition"),
                _ => None,
            };
            if let Some(function) = function {
                let Some(function_name) = function.child_by_field_name("name") else {
                    continue;
                };
                if let Some((_, method)) = VIEW_METHODS
                    .iter()
                    .find(|(n, _)| *n == file.text(function_name))
                {
                    methods.push((method.clone(), function));
                }
                continue;
            }

            // `methods = ["GET"]` and `decorators = [login_required]` class attributes.
            if statement.kind() != "expression_statement" {
                continue;
            }
            let Some(assignment) = statement
                .named_child(0)
                .filter(|n| n.kind() == "assignment")
            else {
                continue;
            };
            let (Some(left), Some(right)) = (
                assignment.child_by_field_name("left"),
                assignment.child_by_field_name("right"),
            ) else {
                continue;
            };
            match file.text(left) {
                "methods" => {
                    declared_methods = Some(
                        list_strings(file, right, &Constants::default())
                            .iter()
                            .filter_map(|m| m.parse::<HttpMethod>().ok())
                            .collect(),
                    );
                }
                "decorators" => {
                    auth = (0..right.named_child_count() as u32)
                        .filter_map(|i| right.named_child(i))
                        .find_map(|d| auth_from_decorator_name(file.text(d)));
                }
                _ => {}
            }
        }

        if auth.is_none() {
            if let Some(decorated) = decorated {
                let mut c = decorated.walk();
                auth = decorated
                    .children(&mut c)
                    .filter(|d| d.kind() == "decorator")
                    .filter_map(|d| d.named_child(0))
                    .find_map(|e| auth_from_decorator_name(file.text(e)));
            }
        }

        if let Some(declared) = declared_methods {
            methods.retain(|(m, _)| declared.contains(m));
        }
        if methods.is_empty() {
            continue;
        }

        let class_name = file.text(name).to_string();
        sink.router(RouterFact {
            symbol: SymbolId::new(file.path.clone(), class_name.clone()),
            prefix: PathTemplate::empty(),
            group: None,
            is_app_root: false,
            factory: None,
            implicit: false,
            span: file.span(name),
        });
        for (method, function) in methods {
            let mut fact = RouteFact::new(
                SymbolRef::new(file.path.clone(), class_name.clone()),
                method,
                PathTemplate::empty(),
                file.span(function),
            );
            fact.auth = auth.clone();
            // The rule's own path parameters are not known here, so nothing is filtered
            // against them; a view method rarely reads a path parameter back out of
            // `request.args` anyway.
            RequestUsage::of(file, function, &[], &REQUEST).apply(&mut fact);
            fact.summary = docstring(file, function);
            sink.route(fact);
        }
        names.insert(class_name);
    }

    names
}

/// How Flask spells the request object's parts.
const REQUEST: RequestDialect = RequestDialect {
    query: &["request.args", "request.values"],
    headers: &["request.headers"],
    meta_headers: &[],
    bodies: &[
        ("request.get_json", "application/json"),
        ("request.json", "application/json"),
        ("request.form", "application/x-www-form-urlencoded"),
        ("request.files", "multipart/form-data"),
        ("request.get_data", "text/plain"),
        ("request.data", "text/plain"),
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::RegistrationGraph;
    use crate::index::SourceIndex;
    use std::collections::BTreeSet;

    fn discover(files: &[(&str, &str)]) -> (Vec<String>, Vec<String>) {
        let mut index = SourceIndex::new().unwrap();
        let mut sink = FactSink::new();

        for (path, source) in files {
            let parsed = index
                .parse(*path, Language::Python, source.to_string())
                .unwrap();
            FlaskAdapter.extract(&parsed, &mut sink);
        }
        let mut warnings = sink.warnings.clone();

        let known: BTreeSet<PathBuf> = files.iter().map(|(p, _)| PathBuf::from(p)).collect();
        let graph = RegistrationGraph::build(sink, &known);
        let resolution = graph.resolve();
        warnings.extend(resolution.warnings.iter().map(|w| w.to_string()));

        let mut routes: Vec<String> = resolution
            .routes
            .iter()
            .map(|r| {
                format!(
                    "{} {}{}",
                    r.method,
                    r.path.render(ParamStyle::Braces),
                    if r.orphaned { " (orphan)" } else { "" }
                )
            })
            .collect();
        routes.sort();
        (routes, warnings)
    }

    fn single(source: &str) -> Vec<String> {
        discover(&[("app.py", source)]).0
    }

    fn facts(source: &str) -> FactSink {
        let mut index = SourceIndex::new().unwrap();
        let parsed = index
            .parse("app.py", Language::Python, source.to_string())
            .unwrap();
        let mut sink = FactSink::new();
        FlaskAdapter.extract(&parsed, &mut sink);
        sink
    }

    #[test]
    fn a_route_on_the_app_with_flask_converters() {
        let routes = single(
            "from flask import Flask\n\
             app = Flask(__name__)\n\
             @app.route(\"/users/<int:user_id>\")\n\
             def user(user_id): ...\n",
        );
        assert_eq!(routes, vec!["GET /users/{user_id}"]);
    }

    #[test]
    fn methods_come_from_the_keyword_and_default_to_get() {
        let routes = single(
            "from flask import Flask\n\
             app = Flask(__name__)\n\
             @app.route(\"/users\", methods=[\"GET\", \"POST\"])\n\
             def users(): ...\n\
             @app.route(\"/ping\")\n\
             def ping(): ...\n",
        );
        assert_eq!(routes, vec!["GET /ping", "GET /users", "POST /users"]);
    }

    #[test]
    fn method_shortcuts_are_recognised() {
        let routes = single(
            "from flask import Flask\n\
             app = Flask(__name__)\n\
             @app.get(\"/a\")\n\
             def a(): ...\n\
             @app.delete(\"/a\")\n\
             def b(): ...\n",
        );
        assert_eq!(routes, vec!["DELETE /a", "GET /a"]);
    }

    #[test]
    fn a_registration_prefix_replaces_the_blueprint_prefix_across_files() {
        let (routes, _) = discover(&[
            (
                "app/users.py",
                "from flask import Blueprint\n\
                 bp = Blueprint(\"users\", __name__, url_prefix=\"/users\")\n\
                 @bp.get(\"/<int:id>\")\n\
                 def get(id): ...\n",
            ),
            (
                "app/__init__.py",
                "from flask import Flask\n\
                 from .users import bp as users_bp\n\
                 def create_app():\n\
                 \x20   app = Flask(__name__)\n\
                 \x20   app.register_blueprint(users_bp, url_prefix=\"/api/v1/users\")\n\
                 \x20   return app\n",
            ),
        ]);
        // Flask semantics: the `url_prefix` at registration overrides the blueprint's own
        // `/users`; composing them would give `/api/v1/users/users/{id}`, which is not
        // what Werkzeug serves.
        assert_eq!(routes, vec!["GET /api/v1/users/{id}"]);
    }

    #[test]
    fn without_a_registration_prefix_the_blueprint_prefix_applies() {
        let routes = single(
            "from flask import Flask, Blueprint\n\
             app = Flask(__name__)\n\
             bp = Blueprint(\"users\", __name__, url_prefix=\"/users\")\n\
             @bp.get(\"/<int:id>\")\n\
             def get(id): ...\n\
             app.register_blueprint(bp)\n",
        );
        assert_eq!(routes, vec!["GET /users/{id}"]);
    }

    #[test]
    fn a_blueprint_nested_in_a_blueprint_is_composed_too() {
        let routes = single(
            "from flask import Flask, Blueprint\n\
             app = Flask(__name__)\n\
             parent = Blueprint(\"parent\", __name__, url_prefix=\"/parent\")\n\
             child = Blueprint(\"child\", __name__)\n\
             @child.route(\"/leaf\")\n\
             def leaf(): ...\n\
             parent.register_blueprint(child, url_prefix=\"/child\")\n\
             app.register_blueprint(parent)\n",
        );
        assert_eq!(routes, vec!["GET /parent/child/leaf"]);
    }

    #[test]
    fn an_unregistered_blueprint_is_an_orphan() {
        let (routes, warnings) = single_with_warnings(
            "from flask import Flask, Blueprint\n\
             app = Flask(__name__)\n\
             stray = Blueprint(\"stray\", __name__, url_prefix=\"/stray\")\n\
             @stray.route(\"/x\")\n\
             def x(): ...\n",
        );
        assert_eq!(routes, vec!["GET /stray/x (orphan)"]);
        assert!(warnings.iter().any(|w| w.contains("never mounted")));
    }

    fn single_with_warnings(source: &str) -> (Vec<String>, Vec<String>) {
        discover(&[("app.py", source)])
    }

    #[test]
    fn an_unfoldable_prefix_is_a_visible_gap() {
        let routes = single(
            "from flask import Flask, Blueprint\n\
             from .config import settings\n\
             app = Flask(__name__)\n\
             bp = Blueprint(\"b\", __name__)\n\
             @bp.route(\"/x\")\n\
             def x(): ...\n\
             app.register_blueprint(bp, url_prefix=settings.PREFIX)\n",
        );
        assert_eq!(routes, vec!["GET /?/x"]);
    }

    #[test]
    fn add_url_rule_with_a_function() {
        let routes = single(
            "from flask import Flask\n\
             app = Flask(__name__)\n\
             def index(): ...\n\
             app.add_url_rule(\"/\", \"index\", index, methods=[\"GET\", \"POST\"])\n",
        );
        assert_eq!(routes, vec!["GET /", "POST /"]);
    }

    #[test]
    fn a_method_view_yields_one_route_per_implemented_method() {
        let routes = single(
            "from flask import Flask\n\
             from flask.views import MethodView\n\
             app = Flask(__name__)\n\
             class UserAPI(MethodView):\n\
             \x20   def get(self, id): ...\n\
             \x20   def put(self, id): ...\n\
             \x20   def helper(self): ...\n\
             app.add_url_rule(\"/users/<int:id>\", view_func=UserAPI.as_view(\"user\"))\n",
        );
        assert_eq!(routes, vec!["GET /users/{id}", "PUT /users/{id}"]);
    }

    #[test]
    fn a_view_class_in_another_file_is_linked_through_the_import() {
        let (routes, _) = discover(&[
            (
                "app/views.py",
                "from flask.views import MethodView\n\
                 class NoteAPI(MethodView):\n\
                 \x20   def get(self, note_id): ...\n\
                 \x20   def put(self, note_id): ...\n\
                 \x20   def delete(self, note_id): ...\n",
            ),
            (
                "app/__init__.py",
                "from flask import Flask\n\
                 from .views import NoteAPI\n\
                 app = Flask(__name__)\n\
                 note_view = NoteAPI.as_view(\"note\")\n\
                 app.add_url_rule(\"/notes/<int:note_id>\", view_func=note_view)\n\
                 app.add_url_rule(\"/notes\", view_func=note_view, methods=[\"GET\"])\n",
            ),
        ]);
        assert_eq!(
            routes,
            vec![
                "DELETE /notes/{note_id}",
                "GET /notes",
                "GET /notes/{note_id}",
                "PUT /notes/{note_id}",
            ],
            "`methods=` narrows the second rule to GET"
        );
    }

    #[test]
    fn a_view_class_that_cannot_be_found_is_reported_not_guessed() {
        let (routes, warnings) = single_with_warnings(
            "from flask import Flask\n\
             from somewhere import UserAPI\n\
             app = Flask(__name__)\n\
             app.add_url_rule(\"/users\", view_func=UserAPI.as_view(\"user\"))\n",
        );
        assert!(routes.is_empty());
        assert!(
            warnings.iter().any(|w| w.contains("UserAPI")),
            "{warnings:?}"
        );
    }

    #[test]
    fn flask_restful_resources_are_routes_under_the_api_prefix() {
        let routes = single(
            "from flask import Flask\n\
             from flask_restful import Api, Resource\n\
             app = Flask(__name__)\n\
             api = Api(app, prefix=\"/api\")\n\
             class Todo(Resource):\n\
             \x20   def get(self, id): ...\n\
             \x20   def delete(self, id): ...\n\
             api.add_resource(Todo, \"/todos/<int:id>\", \"/t/<int:id>\")\n",
        );
        assert_eq!(
            routes,
            vec![
                "DELETE /api/t/{id}",
                "DELETE /api/todos/{id}",
                "GET /api/t/{id}",
                "GET /api/todos/{id}"
            ]
        );
    }

    #[test]
    fn flask_restx_namespaces_route_decorated_classes() {
        let routes = single(
            "from flask import Flask\n\
             from flask_restx import Api, Namespace, Resource\n\
             app = Flask(__name__)\n\
             api = Api(app)\n\
             ns = Namespace(\"users\", path=\"/users\")\n\
             @ns.route(\"/<int:id>\")\n\
             class User(Resource):\n\
             \x20   def get(self, id): ...\n\
             api.add_namespace(ns, path=\"/v1/users\")\n",
        );
        // `add_namespace(path=)` overrides the namespace's own `path`.
        assert_eq!(routes, vec!["GET /v1/users/{id}"]);
    }

    #[test]
    fn query_headers_and_body_are_read_from_the_handler() {
        let sink = facts(
            "from flask import Flask, request\n\
             app = Flask(__name__)\n\
             @app.post(\"/search/<kind>\")\n\
             def search(kind):\n\
             \x20   q = request.args.get(\"q\", \"\")\n\
             \x20   page = request.args[\"page\"]\n\
             \x20   kind = request.args.get(\"kind\")\n\
             \x20   token = request.headers.get(\"X-Token\")\n\
             \x20   data = request.get_json()\n\
             \x20   name = data[\"name\"]\n\
             \x20   age = data.get(\"age\")\n\
             \x20   return {}\n",
        );
        let route = &sink.routes[0];
        let query: Vec<_> = route
            .query_params
            .iter()
            .map(|p| (p.name.as_str(), p.required))
            .collect();
        assert_eq!(
            query,
            vec![("q", false), ("page", true)],
            "path params are not repeated"
        );
        assert_eq!(route.headers[0].name, "X-Token");
        let body = route.body.as_ref().unwrap();
        assert_eq!(body.content_type, "application/json");
        assert_eq!(
            body.example,
            Some(serde_json::json!({ "name": "", "age": "" }))
        );
    }

    #[test]
    fn a_form_read_is_a_form_body() {
        let sink = facts(
            "from flask import Flask, request\n\
             app = Flask(__name__)\n\
             @app.post(\"/login\")\n\
             def login():\n\
             \x20   user = request.form[\"username\"]\n\
             \x20   return {}\n",
        );
        let body = sink.routes[0].body.as_ref().unwrap();
        assert_eq!(body.content_type, "application/x-www-form-urlencoded");
    }

    #[test]
    fn auth_decorators_become_requirements() {
        let sink = facts(
            "from flask import Flask\n\
             app = Flask(__name__)\n\
             @app.get(\"/me\")\n\
             @login_required\n\
             def me(): ...\n\
             @app.get(\"/secure\")\n\
             @jwt_required()\n\
             def secure(): ...\n\
             @app.get(\"/basic\")\n\
             @basic_auth.login_required\n\
             def basic(): ...\n\
             @app.get(\"/open\")\n\
             @cache.cached(timeout=50)\n\
             def open_(): ...\n",
        );
        let auth: Vec<_> = sink.routes.iter().map(|r| r.auth.clone()).collect();
        assert_eq!(
            auth,
            vec![
                Some(AuthRequirement::Cookie {
                    name: "session".into()
                }),
                Some(AuthRequirement::Bearer { format: None }),
                Some(AuthRequirement::Basic),
                None,
            ]
        );
    }

    #[test]
    fn a_class_level_decorators_attribute_guards_every_method() {
        let sink = facts(
            "from flask import Flask\n\
             from flask.views import MethodView\n\
             app = Flask(__name__)\n\
             class Admin(MethodView):\n\
             \x20   decorators = [login_required]\n\
             \x20   methods = [\"GET\"]\n\
             \x20   def get(self): ...\n\
             \x20   def post(self): ...\n\
             app.add_url_rule(\"/admin\", view_func=Admin.as_view(\"admin\"))\n",
        );
        let class_routes: Vec<_> = sink
            .routes
            .iter()
            .filter(|r| r.router.name == "Admin")
            .collect();
        assert_eq!(class_routes.len(), 1, "`methods` narrows the class");
        assert!(class_routes[0].auth.is_some());
    }

    #[test]
    fn the_docstring_is_the_summary_and_the_blueprint_name_the_group() {
        let (routes, _) = discover(&[(
            "app.py",
            "from flask import Flask, Blueprint\n\
             app = Flask(__name__)\n\
             bp = Blueprint(\"items\", __name__)\n\
             @bp.get(\"/items\")\n\
             def items():\n\
             \x20   \"\"\"List items.\"\"\"\n\
             \x20   return []\n\
             app.register_blueprint(bp)\n",
        )]);
        assert_eq!(routes, vec!["GET /items"]);

        let sink = facts(
            "from flask import Flask, Blueprint\n\
             app = Flask(__name__)\n\
             bp = Blueprint(\"items\", __name__)\n\
             @bp.get(\"/items\")\n\
             def items():\n\
             \x20   \"\"\"List items.\"\"\"\n\
             \x20   return []\n",
        );
        assert_eq!(sink.routes[0].summary.as_deref(), Some("List items."));
        assert_eq!(sink.routers[1].group.as_deref(), Some("items"));
    }

    #[test]
    fn a_trailing_slash_rule_keeps_its_slash() {
        let routes = single(
            "from flask import Flask, Blueprint\n\
             app = Flask(__name__)\n\
             bp = Blueprint(\"u\", __name__, url_prefix=\"/users\")\n\
             @bp.route(\"/\")\n\
             def list_(): ...\n\
             app.register_blueprint(bp)\n",
        );
        assert_eq!(routes, vec!["GET /users/"]);
    }

    #[test]
    fn detection_weighs_an_import_more_than_a_manifest_entry() {
        use crate::project::ProjectContext;
        use std::fs;
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("requirements.txt"), "flask\n").unwrap();
        fs::write(
            dir.path().join("app.py"),
            "from flask import Flask\napp = Flask(__name__)\n",
        )
        .unwrap();
        let project = ProjectContext::scan(dir.path()).unwrap();
        let detection = FlaskAdapter.detect(&project);
        assert_eq!(detection.score, 4);
    }
}
