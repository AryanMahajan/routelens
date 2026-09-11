//! Express.
//!
//! Recognises `express()` and `Router()` declarations, `.get/.post/...` registrations,
//! `.route(path).get().post()` chains, and `.use(prefix, router)` mounts — then hands them
//! to the graph. It never joins a path.
//!
//! Express has no signatures to read, so what a handler *reads* off `req` stands in for
//! one: `req.query.limit` becomes a query parameter, `req.body.email` a body field.

use super::js::{
    arguments, auth_from_middleware, callee, collect_exports, collect_imports, reference,
    require_source, top_level_function, Constants, RequestUsage,
};
use super::{Detection, FrameworkAdapter, UNSPECIFIED_METHODS};
use crate::facts::{FactSink, MountFact, RouteFact, RouterFact, Span, SymbolId, SymbolRef};
use crate::index::{walk, ParsedFile};
use crate::project::{Language, ProjectContext};
use rl_model::{AuthRequirement, BodySchema, HttpMethod, ParamSpec, PathTemplate};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use tree_sitter::Node;

const METHODS: &[(&str, HttpMethod)] = &[
    ("get", HttpMethod::Get),
    ("post", HttpMethod::Post),
    ("put", HttpMethod::Put),
    ("patch", HttpMethod::Patch),
    ("delete", HttpMethod::Delete),
    ("head", HttpMethod::Head),
    ("options", HttpMethod::Options),
    ("trace", HttpMethod::Trace),
];

/// Router-file names that say nothing about what the router serves.
const MEANINGLESS_STEMS: &[&str] = &["index", "app", "server", "main", "routes", "router"];

/// Middleware factories that serve files rather than routes, so a prefixed `.use` of one
/// is not a router that static analysis lost.
const STATIC_SERVERS: &[&str] = &["express.static", "serveStatic", "static"];

pub struct ExpressAdapter;

impl FrameworkAdapter for ExpressAdapter {
    fn id(&self) -> &'static str {
        "express"
    }

    fn languages(&self) -> &[Language] {
        &[Language::JavaScript, Language::TypeScript]
    }

    fn detect(&self, project: &ProjectContext) -> Detection {
        let mut detection = Detection::none();

        if project
            .manifest("package.json")
            .is_some_and(|text| text.contains("\"express\""))
        {
            detection.add(1, "`express` is declared in package.json");
        }

        // An actual import is much stronger evidence than a manifest entry.
        let mut imported_in = None;
        for path in project
            .files_of(Language::JavaScript)
            .chain(project.files_of(Language::TypeScript))
        {
            let Ok(source) = project.read(path) else {
                continue;
            };
            if imports_express(&source) {
                imported_in = Some(path.clone());
                break;
            }
        }
        if let Some(path) = imported_in {
            detection.add(
                3,
                format!("express is imported in {}", crate::project::display(&path)),
            );
        }

        detection
    }

    fn candidate_files(&self, project: &ProjectContext) -> Vec<PathBuf> {
        project
            .files_of(Language::JavaScript)
            .chain(project.files_of(Language::TypeScript))
            .filter(|path| {
                let text = path.to_string_lossy().replace('\\', "/");
                let name = text.rsplit('/').next().unwrap_or("");
                !text.contains("/tests/")
                    && !text.contains("/test/")
                    && !text.contains("/__tests__/")
                    && !text.starts_with("tests/")
                    && !text.starts_with("test/")
                    && !name.contains(".test.")
                    && !name.contains(".spec.")
                    && !name.ends_with(".d.ts")
                    && !name.ends_with(".config.js")
                    && !name.ends_with(".config.ts")
                    && !name.ends_with(".config.mjs")
            })
            .cloned()
            .collect()
    }

    fn extract(&self, file: &ParsedFile, sink: &mut FactSink) {
        let constants = Constants::collect(file);

        // Names bound to packages: `cors`, `helmet`, `morgan`. A `.use` of one of these is
        // middleware, not a router, and there is nothing in the project to trace it to.
        let mut externals = BTreeSet::new();
        for import in collect_imports(file) {
            if !is_project_relative(&import.source) {
                externals.insert(import.local_name.clone());
            }
            sink.import(import);
        }
        for export in collect_exports(file) {
            sink.export(export);
        }

        let mut extractor = Extractor {
            file,
            constants: &constants,
            externals: &externals,
            router_auth: BTreeMap::new(),
            sink,
        };

        walk(file.root(), &mut |node| match node.kind() {
            "variable_declarator" | "assignment_expression" => extractor.declaration(node),
            "call_expression" => extractor.call(node),
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

fn imports_express(source: &str) -> bool {
    [
        "require('express')",
        "require(\"express\")",
        "from 'express'",
        "from \"express\"",
    ]
    .iter()
    .any(|needle| source.contains(needle))
}

fn is_project_relative(source: &str) -> bool {
    source.starts_with('.') || source.starts_with("@/") || source.starts_with("~/")
}

struct Extractor<'s, 'f> {
    file: &'f ParsedFile,
    constants: &'s Constants,
    externals: &'s BTreeSet<String>,
    /// `router.use(requireAuth)` applies to every route registered on it afterwards.
    router_auth: BTreeMap<String, AuthRequirement>,
    sink: &'s mut FactSink,
}

impl Extractor<'_, '_> {
    fn span(&self, node: Node<'_>) -> Span {
        self.file.span(node)
    }

    fn text(&self, node: Node<'_>) -> &str {
        self.file.text(node)
    }

    /// `const app = express()`, `const router = express.Router()`, `router = new Router()`.
    fn declaration(&mut self, node: Node<'_>) {
        let (name_field, value_field) = if node.kind() == "variable_declarator" {
            ("name", "value")
        } else {
            ("left", "right")
        };
        let (Some(name), Some(value)) = (
            node.child_by_field_name(name_field),
            node.child_by_field_name(value_field),
        ) else {
            return;
        };
        if name.kind() != "identifier" {
            return;
        }

        // `var app = module.exports = express()`: the value is behind an assignment, and
        // the assignment is also an export.
        let mut value = value;
        while value.kind() == "assignment_expression" {
            let (Some(left), Some(right)) = (
                value.child_by_field_name("left"),
                value.child_by_field_name("right"),
            ) else {
                return;
            };
            if self.text(left) == "module.exports" {
                self.sink.export(crate::facts::ExportFact {
                    module: self.file.path.clone(),
                    exported: "default".to_string(),
                    local: self.text(name).to_string(),
                });
            }
            value = right;
        }

        let constructed = match value.kind() {
            "call_expression" => value,
            "new_expression" => value,
            _ => return,
        };
        let callee_text = match constructed.kind() {
            "new_expression" => constructed
                .child_by_field_name("constructor")
                .map(|c| self.text(c).to_string()),
            _ => constructed
                .child_by_field_name("function")
                .map(|f| self.text(f).to_string()),
        };
        let Some(callee_text) = callee_text else {
            return;
        };

        let is_app_root = callee_text == "express";
        let is_router = matches!(callee_text.as_str(), "express.Router" | "Router")
            || callee_text.ends_with(".Router");
        if !is_app_root && !is_router {
            return;
        }

        self.sink.router(RouterFact {
            symbol: SymbolId::new(self.file.path.clone(), self.text(name)),
            prefix: PathTemplate::empty(),
            group: if is_app_root {
                None
            } else {
                group_from_file(&self.file.path)
            },
            is_app_root,
            span: self.span(node),
        });
    }

    fn call(&mut self, node: Node<'_>) {
        let Some((Some(receiver), method)) = callee(self.file, node) else {
            return;
        };

        if method == "use" {
            self.mount(node, receiver);
            return;
        }

        let methods: Vec<HttpMethod> = if method == "all" {
            UNSPECIFIED_METHODS.to_vec()
        } else {
            match METHODS.iter().find(|(name, _)| *name == method) {
                Some((_, m)) => vec![m.clone()],
                None => return,
            }
        };

        self.route(node, receiver, methods, method == "all");
    }

    /// Follow a chain back to the router it started from.
    ///
    /// `app.route("/x").get(a).post(b)`: the `.post` call's receiver is the `.get` call,
    /// whose receiver is the `.route` call, whose receiver is `app`. Every method returns
    /// the router, so `app.get("/a", h).post("/b", h)` is a chain too.
    fn unwind<'a>(&mut self, receiver: Node<'a>) -> Option<(Node<'a>, Option<PathTemplate>)> {
        if receiver.kind() != "call_expression" {
            return Some((receiver, None));
        }
        let (Some(inner), method) = callee(self.file, receiver)? else {
            return None;
        };
        if method == "route" {
            let path = arguments(receiver)
                .first()
                .map(|p| self.constants.path_value(self.file, *p));
            return Some((inner, path));
        }
        if method == "use" || method == "all" || METHODS.iter().any(|(m, _)| *m == method) {
            return self.unwind(inner);
        }
        None
    }

    /// Whether a receiver chain passes through `.route(path)`, without reporting anything.
    fn chained_route_path(&self, receiver: Node<'_>) -> Option<bool> {
        let mut node = receiver;
        loop {
            if node.kind() != "call_expression" {
                return Some(false);
            }
            let (Some(inner), method) = callee(self.file, node)? else {
                return None;
            };
            if method == "route" {
                return Some(true);
            }
            node = inner;
        }
    }

    fn router_ref(&mut self, receiver: Node<'_>) -> Option<String> {
        let (name, import) = reference(self.file, receiver)?;
        if let Some(import) = import {
            self.sink.import(import);
        }
        Some(name)
    }

    fn route(&mut self, call: Node<'_>, receiver: Node<'_>, methods: Vec<HttpMethod>, any: bool) {
        // Before anything is reported about the receiver, make sure this is a route at all.
        let has_path = arguments(call)
            .first()
            .is_some_and(|first| self.is_path_argument(*first, true))
            || matches!(self.chained_route_path(receiver), Some(true));
        if !has_path {
            return;
        }

        let Some((router_node, chained_path)) = self.unwind(receiver) else {
            self.sink.warn(format!(
                "{}:{}: could not tell which router `{}` registers on",
                self.file.display_path(),
                self.span(call).line,
                self.text(receiver)
            ));
            return;
        };
        let Some(router) = self.router_ref(router_node) else {
            self.sink.warn(format!(
                "{}:{}: could not read `{}` as a router",
                self.file.display_path(),
                self.span(call).line,
                self.text(router_node)
            ));
            return;
        };

        let args = arguments(call);
        let (paths, handlers): (Vec<PathTemplate>, &[Node<'_>]) = match chained_path {
            Some(path) => (vec![path], &args[..]),
            None => match args.split_first() {
                Some((first, rest)) if self.is_path_argument(*first, true) => {
                    (self.paths_of(*first), rest)
                }
                // Not a route. `.get` and `.delete` are ordinary method names —
                // `res.get("Content-Type")`, `cache.delete(key)` — and the only thing that
                // separates a route registration from those is a path. Express does allow
                // `router.get(handler)` with no path, and that form is given up so the
                // far more common non-route `.get` stays silent.
                _ => return,
            },
        };

        let mut usage = RequestUsage::default();
        let mut auth = None;
        for handler in handlers {
            self.inspect_handler(*handler, &mut usage, &mut auth);
        }
        if auth.is_none() {
            auth = self.router_auth.get(&router).cloned();
        }

        // The span of `.post` itself, not of the whole expression: in a chain written one
        // method per line, that is the line the developer wants to land on.
        let span = call
            .child_by_field_name("function")
            .and_then(|f| f.child_by_field_name("property"))
            .map(|p| self.span(p))
            .unwrap_or_else(|| self.span(call));

        for path in paths {
            let mut fact = RouteFact::new(
                SymbolRef::new(self.file.path.clone(), router.clone()),
                methods[0].clone(),
                path,
                span,
            );
            fact.methods = methods.clone();
            fact.query_params = usage.query.iter().map(ParamSpec::new).collect();
            fact.headers = usage.headers.iter().map(ParamSpec::new).collect();
            fact.body = usage.reads_body.then(|| body_schema(&usage.body));
            fact.auth = auth.clone();
            super::js::promote_authorization_header(&mut fact.headers, &mut fact.auth);
            if any {
                fact.summary = Some("registered with `.all`, so any method reaches it".into());
            }
            self.sink.route(fact);
        }
    }

    /// `app.use("/api/users", usersRouter)`, `app.use(router)`, `router.use(requireAuth)`.
    fn mount(&mut self, call: Node<'_>, receiver: Node<'_>) {
        let Some((parent_node, _)) = self.unwind(receiver) else {
            return;
        };
        let Some(parent) = self.router_ref(parent_node) else {
            return;
        };

        let args = arguments(call);
        let (prefixes, children): (Vec<PathTemplate>, &[Node<'_>]) = match args.split_first() {
            Some((first, rest)) if self.is_path_argument(*first, false) => {
                (self.paths_of(*first), rest)
            }
            _ => (vec![PathTemplate::empty()], &args[..]),
        };
        let prefixed = !prefixes.iter().all(|p| p.segments.is_empty());

        // `app.use("/admin", requireAuth, adminRouter)`: the middleware guards the mount.
        let mut auth = None;
        for child in children {
            self.mount_child(call, &parent, &prefixes, prefixed, *child, &mut auth);
        }
    }

    fn mount_child(
        &mut self,
        call: Node<'_>,
        parent: &str,
        prefixes: &[PathTemplate],
        prefixed: bool,
        child: Node<'_>,
        auth: &mut Option<AuthRequirement>,
    ) {
        match child.kind() {
            "array" => {
                for element in named_children(child) {
                    self.mount_child(call, parent, prefixes, prefixed, element, auth);
                }
            }
            "identifier" => {
                let name = self.text(child).to_string();
                if self.externals.contains(&name) {
                    return; // `app.use(cors)` — a package, not a router
                }
                if let Some(found) = auth_from_middleware(&name) {
                    if prefixed {
                        *auth = Some(found);
                    } else {
                        // `router.use(requireAuth)`: applies to the routes that follow.
                        self.router_auth.insert(parent.to_string(), found);
                    }
                    return;
                }
                self.emit_mounts(call, parent, prefixes, &name, None, auth.clone());
            }
            "member_expression" => {
                if let Some((name, import)) = reference(self.file, child) {
                    self.emit_mounts(call, parent, prefixes, &name, import, auth.clone());
                }
            }
            "call_expression" => {
                if require_source(self.file, child).is_some() {
                    if let Some((name, import)) = reference(self.file, child) {
                        self.emit_mounts(call, parent, prefixes, &name, import, auth.clone());
                    }
                    return;
                }
                if auth.is_none() {
                    // `passport.authenticate("jwt")` ahead of the router.
                    *auth = auth_from_middleware(self.text(child));
                    if auth.is_some() {
                        return;
                    }
                }
                // `app.use("/api", createRouter())` — the router is built by a call, and
                // following one means executing it. Say so, rather than dropping it. A bare
                // `app.use(cors())` is middleware and stays quiet.
                if prefixed {
                    let callee_text = child
                        .child_by_field_name("function")
                        .map(|f| self.text(f).to_string())
                        .unwrap_or_default();
                    if !STATIC_SERVERS.contains(&callee_text.as_str()) {
                        self.sink.warn(format!(
                            "{}:{}: `{}` is mounted at {} but comes from a call, which static \
                             analysis cannot follow",
                            self.file.display_path(),
                            self.span(call).line,
                            self.text(child),
                            prefixes[0]
                        ));
                    }
                }
            }
            _ => {} // inline middleware functions
        }
    }

    fn emit_mounts(
        &mut self,
        call: Node<'_>,
        parent: &str,
        prefixes: &[PathTemplate],
        child: &str,
        import: Option<crate::facts::ImportFact>,
        auth: Option<AuthRequirement>,
    ) {
        if let Some(import) = import {
            self.sink.import(import);
        }
        for prefix in prefixes {
            self.sink.mount(MountFact {
                parent: SymbolRef::new(self.file.path.clone(), parent),
                child: SymbolRef::new(self.file.path.clone(), child),
                prefix: prefix.clone(),
                group: None,
                auth: auth.clone(),
                span: self.span(call),
            });
        }
    }

    /// Whether a first argument is a path rather than a handler or router.
    ///
    /// Literals and arrays of them obviously are. An identifier is when it folds to a
    /// constant; otherwise it is a handler (`app.get(handler)`) or a router
    /// (`app.use(router)`). A member expression is the ambiguous case — `app.use(routes.users)`
    /// against `app.use(config.PREFIX, router)` — and is taken as a path only when it reads
    /// like configuration, which is a heuristic and named as one.
    fn is_path_argument(&self, node: Node<'_>, for_route: bool) -> bool {
        match node.kind() {
            // A route path starts with `/`; `res.get("Content-Type")` does not.
            "string" | "template_string" | "binary_expression" => self
                .constants
                .string_value(self.file, node)
                .is_none_or(|text| text.starts_with('/')),
            "regex" => true,
            "array" => named_children(node)
                .first()
                .is_some_and(|first| self.is_path_argument(*first, for_route)),
            "identifier" => {
                let name = self.text(node);
                match self.constants.get(name) {
                    Some(value) => value.starts_with('/'),
                    None => for_route && is_screaming_case(name),
                }
            }
            "member_expression" => looks_like_configuration(self.text(node)),
            _ => false,
        }
    }

    fn paths_of(&self, node: Node<'_>) -> Vec<PathTemplate> {
        if node.kind() == "array" {
            named_children(node)
                .into_iter()
                .map(|element| self.path_of(element))
                .collect()
        } else {
            vec![self.path_of(node)]
        }
    }

    fn path_of(&self, node: Node<'_>) -> PathTemplate {
        match self.constants.string_value(self.file, node) {
            Some(text) => parse_express_path(&text),
            None => self.constants.path_value(self.file, node),
        }
    }

    /// Read what a handler uses, and whether a middleware in the chain looks like auth.
    fn inspect_handler(
        &self,
        node: Node<'_>,
        usage: &mut RequestUsage,
        auth: &mut Option<AuthRequirement>,
    ) {
        match node.kind() {
            "arrow_function" | "function_expression" | "function" => {
                merge(usage, RequestUsage::of(self.file, node));
            }
            "array" => {
                for element in named_children(node) {
                    self.inspect_handler(element, usage, auth);
                }
            }
            "identifier" => {
                let name = self.text(node);
                match top_level_function(self.file, name) {
                    Some(function) => merge(usage, RequestUsage::of(self.file, function)),
                    None => {
                        if auth.is_none() {
                            *auth = auth_from_middleware(name);
                        }
                    }
                }
            }
            // `passport.authenticate("jwt")`, `auth.required`, `controller.list`. The
            // whole call, arguments included: `passport.authenticate("jwt")` names its
            // scheme in the argument, not the callee.
            "call_expression" | "member_expression" if auth.is_none() => {
                *auth = auth_from_middleware(self.text(node));
            }
            _ => {}
        }
    }
}

/// Parse a path in Express's own syntax, both generations of it.
///
/// Express 4 (path-to-regexp 0.x): `:id`, `:id?`, `*`. Express 5 (path-to-regexp 8):
/// `:id`, `{/:op}` for an optional group, `*name` for a named wildcard. The braces are
/// removed and every parameter inside them marked optional — a group can span segments,
/// which the segment-wise parser cannot express, so this is the honest approximation.
fn parse_express_path(text: &str) -> PathTemplate {
    let mut cleaned = String::with_capacity(text.len());
    let mut optional_names = Vec::new();
    let mut depth = 0u32;
    let mut current = String::new();

    let flush = |current: &mut String, depth: u32, optional_names: &mut Vec<String>| {
        if depth > 0 && !current.is_empty() {
            optional_names.push(std::mem::take(current));
        }
        current.clear();
    };

    for c in text.chars() {
        match c {
            '{' => {
                depth += 1;
            }
            '}' => {
                flush(&mut current, depth, &mut optional_names);
                depth = depth.saturating_sub(1);
            }
            ':' if depth > 0 => {
                flush(&mut current, depth, &mut optional_names);
                cleaned.push(c);
            }
            c if depth > 0 && (c.is_alphanumeric() || c == '_') => {
                current.push(c);
                cleaned.push(c);
            }
            c => {
                flush(&mut current, depth, &mut optional_names);
                cleaned.push(c);
            }
        }
    }

    let mut template = PathTemplate::parse(&cleaned, rl_model::ParamStyle::Colon);
    for segment in &mut template.segments {
        match segment {
            rl_model::PathSegment::Param { name, optional, .. }
                if optional_names.contains(name) =>
            {
                *optional = true;
            }
            // `*path` — Express 5's named wildcard.
            rl_model::PathSegment::Literal { value }
                if value.starts_with('*') && value.len() > 1 =>
            {
                *segment = rl_model::PathSegment::Param {
                    name: value[1..].to_string(),
                    ty: Some(rl_model::TypeHint::Path),
                    catch_all: true,
                    optional: false,
                };
            }
            _ => {}
        }
    }
    template
}

fn merge(into: &mut RequestUsage, from: RequestUsage) {
    for name in from.query {
        if !into.query.contains(&name) {
            into.query.push(name);
        }
    }
    for name in from.body {
        if !into.body.contains(&name) {
            into.body.push(name);
        }
    }
    for name in from.headers {
        if !into.headers.contains(&name) {
            into.headers.push(name);
        }
    }
    into.reads_body |= from.reads_body;
}

fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    (0..node.named_child_count() as u32)
        .filter_map(|i| node.named_child(i))
        .filter(|n| n.kind() != "comment")
        .collect()
}

fn is_screaming_case(name: &str) -> bool {
    name.len() > 1
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn looks_like_configuration(text: &str) -> bool {
    if text.starts_with("process.env.") {
        return true;
    }
    let object = text.split('.').next().unwrap_or("");
    let last = text.rsplit('.').next().unwrap_or("");
    matches!(
        object.to_ascii_lowercase().as_str(),
        "config" | "settings" | "constants" | "env" | "paths" | "prefixes"
    ) || is_screaming_case(last)
}

/// `routes/users.js` → `users`; `routes/index.js` → nothing useful.
fn group_from_file(path: &std::path::Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    // `users.routes.ts`, `users.router.js`
    let stem = stem.split('.').next().unwrap_or(stem);
    if MEANINGLESS_STEMS.contains(&stem) {
        // `routes/users/index.js` — the directory says it instead.
        let dir = path.parent()?.file_name()?.to_str()?;
        return (!MEANINGLESS_STEMS.contains(&dir) && dir != "src").then(|| dir.to_string());
    }
    Some(stem.to_string())
}

/// A body schema listing the fields the handler read, so the editor can pre-fill them.
fn body_schema(fields: &[String]) -> BodySchema {
    let mut schema = BodySchema::json();
    if !fields.is_empty() {
        let properties: serde_json::Map<String, serde_json::Value> = fields
            .iter()
            .map(|f| (f.clone(), serde_json::json!({})))
            .collect();
        schema.schema = Some(serde_json::json!({
            "type": "object",
            "properties": properties,
        }));
        let example: serde_json::Map<String, serde_json::Value> = fields
            .iter()
            .map(|f| (f.clone(), serde_json::Value::String(String::new())))
            .collect();
        schema.example = Some(serde_json::Value::Object(example));
    }
    schema
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::SourceIndex;
    use rl_model::ParamStyle;

    fn extract(path: &str, source: &str) -> FactSink {
        let language = Language::of(std::path::Path::new(path)).unwrap();
        let file = SourceIndex::new()
            .unwrap()
            .parse(path, language, source.to_string())
            .unwrap();
        let mut sink = FactSink::new();
        ExpressAdapter.extract(&file, &mut sink);
        sink
    }

    /// In source order, which for a chain means the order the methods were written.
    fn routes(sink: &FactSink) -> Vec<String> {
        let mut facts: Vec<&RouteFact> = sink.routes.iter().collect();
        facts.sort_by_key(|r| (r.span.line, r.span.column));
        facts
            .iter()
            .flat_map(|r| {
                r.methods.iter().map(move |m| {
                    format!(
                        "{} {} on {}",
                        m,
                        r.path.render(ParamStyle::Colon),
                        r.router.name
                    )
                })
            })
            .collect()
    }

    #[test]
    fn declarations_in_every_spelling() {
        let sink = extract(
            "app.js",
            "const app = express();\n\
             const a = express.Router();\n\
             const b = Router({ mergeParams: true });\n\
             const c = new Router();\n\
             let d;\nd = new express.Router();\n\
             const notARouter = createThing();\n",
        );
        let names: Vec<(&str, bool)> = sink
            .routers
            .iter()
            .map(|r| (r.symbol.name.as_str(), r.is_app_root))
            .collect();
        assert_eq!(
            names,
            vec![
                ("app", true),
                ("a", false),
                ("b", false),
                ("c", false),
                ("d", false)
            ]
        );
    }

    #[test]
    fn methods_paths_and_params() {
        let sink = extract(
            "routes/users.js",
            "router.get('/', list);\n\
             router.post('/', create);\n\
             router.delete('/:id', remove);\n\
             router.get('/:id/posts/:postId?', h);\n",
        );
        assert_eq!(
            routes(&sink),
            vec![
                "GET / on router",
                "POST / on router",
                "DELETE /:id on router",
                "GET /:id/posts/:postId? on router",
            ]
        );
        let optional = &sink.routes[3].path.segments[2];
        assert!(matches!(
            optional,
            rl_model::PathSegment::Param { optional: true, .. }
        ));
    }

    /// The first thing a scan of the Express repository itself turned up: `res.get`,
    /// `cache.delete` and every other `.get` in a codebase are not routes.
    #[test]
    fn a_get_without_a_path_is_not_a_route() {
        let sink = extract(
            "lib/response.js",
            "const type = res.get('Content-Type');
             cache.delete(key);
             this.set('X', y);
             map.get(id).post(thing);
             router.get(handler);
",
        );
        assert!(sink.routes.is_empty());
        assert!(sink.warnings.is_empty(), "{:?}", sink.warnings);
    }

    #[test]
    fn a_chained_assignment_declares_and_exports_the_app() {
        let sink = extract(
            "index.js",
            "var app = module.exports = express();
",
        );
        assert_eq!(sink.routers[0].symbol.name, "app");
        assert!(sink.routers[0].is_app_root);
        assert_eq!(
            (
                sink.exports[0].exported.as_str(),
                sink.exports[0].local.as_str()
            ),
            ("default", "app")
        );
    }

    #[test]
    fn express_5_path_syntax() {
        let render = |raw: &str| parse_express_path(raw).render(ParamStyle::Colon);
        assert_eq!(render("/user/:id{/:op}"), "/user/:id/:op?");
        assert_eq!(render("/search/{:query}"), "/search/:query?");
        assert_eq!(render("/files/*path"), "/files/*");
        assert_eq!(
            render("/legacy/:id?"),
            "/legacy/:id?",
            "Express 4 still works"
        );
        let wildcard = parse_express_path("/files/*path");
        assert!(matches!(
            &wildcard.segments[1],
            rl_model::PathSegment::Param { name, catch_all: true, .. } if name == "path"
        ));
    }

    #[test]
    fn a_route_chain_shares_its_path() {
        let sink = extract(
            "app.js",
            "app.route('/items/:id').get(show).put(update).delete(remove);\n",
        );
        assert_eq!(
            routes(&sink),
            vec![
                "GET /items/:id on app",
                "PUT /items/:id on app",
                "DELETE /items/:id on app",
            ]
        );
    }

    #[test]
    fn a_method_chain_keeps_each_path() {
        let sink = extract("app.js", "app.get('/a', h).post('/b', h);\n");
        assert_eq!(routes(&sink), vec!["GET /a on app", "POST /b on app"]);
    }

    #[test]
    fn all_expands_to_the_unspecified_methods_and_says_so() {
        let sink = extract("app.js", "app.all('/ping', h);\n");
        assert_eq!(sink.routes.len(), 1);
        assert_eq!(sink.routes[0].methods, UNSPECIFIED_METHODS.to_vec());
        assert!(sink.routes[0].summary.as_deref().unwrap().contains(".all"));
    }

    #[test]
    fn an_array_of_paths_registers_each() {
        let sink = extract("app.js", "app.get(['/a', '/b'], h);\n");
        assert_eq!(routes(&sink), vec!["GET /a on app", "GET /b on app"]);
    }

    #[test]
    fn a_regex_or_env_path_is_an_honest_gap() {
        let sink = extract(
            "app.js",
            "app.get(/^\\/api\\/.*$/, h);\napp.use(process.env.PREFIX, router);\n",
        );
        assert!(!sink.routes[0].path.is_resolved());
        assert!(!sink.mounts[0].prefix.is_resolved());
        assert_eq!(
            sink.mounts[0].prefix.unresolved_exprs(),
            vec!["process.env.PREFIX"]
        );
    }

    #[test]
    fn a_constant_prefix_folds() {
        let sink = extract(
            "app.js",
            "const API = '/api';\nconst V1 = `${API}/v1`;\napp.use(V1, router);\n",
        );
        assert_eq!(sink.mounts[0].prefix.render(ParamStyle::Colon), "/api/v1");
    }

    #[test]
    fn mounts_in_every_spelling() {
        let sink = extract(
            "app.js",
            "const cors = require('cors');\n\
             const users = require('./routes/users');\n\
             app.use(cors());\n\
             app.use(cors);\n\
             app.use(express.json());\n\
             app.use('/api/users', users);\n\
             app.use('/api', require('./routes/orders'));\n\
             app.use(routes.admin);\n\
             app.use('/v', [a, b]);\n\
             app.use('/static', express.static('public'));\n",
        );
        let mounts: Vec<String> = sink
            .mounts
            .iter()
            .map(|m| format!("{} at {}", m.child.name, m.prefix.render(ParamStyle::Colon)))
            .collect();
        assert_eq!(
            mounts,
            vec![
                "users at /api/users",
                "require(\"./routes/orders\") at /api",
                "routes.admin at /",
                "a at /v",
                "b at /v",
            ]
        );
        // The inline require was recorded as an import so the graph can follow it.
        assert!(sink
            .imports
            .iter()
            .any(|i| i.local_name == "require(\"./routes/orders\")"));
        assert!(sink.warnings.is_empty(), "{:?}", sink.warnings);
    }

    #[test]
    fn a_router_from_a_factory_call_is_reported_not_dropped() {
        let sink = extract("app.js", "app.use('/api/v2', createV2Router());\n");
        assert!(sink.mounts.is_empty());
        assert!(sink.warnings[0].contains("createV2Router()"));
        assert!(sink.warnings[0].contains("/api/v2"));
    }

    #[test]
    fn handler_usage_becomes_params_and_body() {
        let sink = extract(
            "app.js",
            "app.post('/users', (req, res) => {\n\
               const { limit } = req.query;\n\
               const email = req.body.email;\n\
               const key = req.get('X-Api-Key');\n\
             });\n",
        );
        let route = &sink.routes[0];
        assert_eq!(route.query_params[0].name, "limit");
        assert_eq!(route.headers[0].name, "X-Api-Key");
        let body = route.body.as_ref().unwrap();
        assert_eq!(body.example.as_ref().unwrap()["email"], "");
    }

    #[test]
    fn auth_middleware_in_the_chain_is_noticed() {
        let sink = extract(
            "app.js",
            "app.get('/me', requireAuth, h);\n\
             app.get('/jwt', passport.authenticate('jwt', { session: false }), h);\n\
             app.get('/open', h);\n",
        );
        assert!(matches!(
            sink.routes[0].auth,
            Some(AuthRequirement::Unknown { ref hint }) if hint == "requireAuth"
        ));
        assert!(matches!(
            sink.routes[1].auth,
            Some(AuthRequirement::Bearer { .. })
        ));
        assert!(sink.routes[2].auth.is_none());
    }

    #[test]
    fn middleware_ahead_of_a_mounted_router_guards_the_mount() {
        let sink = extract(
            "app.js",
            "app.use('/admin', requireAuth, adminRouter);\n\
             app.use('/jwt', passport.authenticate('jwt'), jwtRouter);\n\
             app.use('/open', openRouter);\n",
        );
        assert_eq!(sink.mounts.len(), 3);
        assert!(matches!(
            sink.mounts[0].auth,
            Some(AuthRequirement::Unknown { ref hint }) if hint == "requireAuth"
        ));
        assert!(matches!(
            sink.mounts[1].auth,
            Some(AuthRequirement::Bearer { .. })
        ));
        assert!(sink.mounts[2].auth.is_none());
        assert!(sink.warnings.is_empty(), "{:?}", sink.warnings);
    }

    #[test]
    fn router_level_auth_applies_to_the_routes_after_it() {
        let sink = extract(
            "r.js",
            "router.get('/public', h);\n\
             router.use(requireAuth);\n\
             router.get('/private', h);\n",
        );
        assert!(sink.routes[0].auth.is_none());
        assert!(sink.routes[1].auth.is_some());
        assert!(sink.mounts.is_empty(), "auth middleware is not a mount");
    }

    #[test]
    fn routers_are_grouped_by_file_name() {
        assert_eq!(
            group_from_file(std::path::Path::new("src/routes/users.js")).as_deref(),
            Some("users")
        );
        assert_eq!(
            group_from_file(std::path::Path::new("src/routes/users.routes.ts")).as_deref(),
            Some("users")
        );
        assert_eq!(
            group_from_file(std::path::Path::new("src/routes/orders/index.js")).as_deref(),
            Some("orders")
        );
        assert_eq!(group_from_file(std::path::Path::new("src/app.js")), None);
    }

    /// A class-based controller registers on `this.router`, which needs the class to be
    /// instantiated to know. Reported, not guessed.
    #[test]
    fn a_receiver_that_cannot_be_read_is_reported() {
        let sink = extract("c.ts", "this.router.get('/x', h);\n");
        assert!(sink.routes.is_empty());
        assert!(sink.warnings[0].contains("could not read `this.router`"));

        let sink = extract("c.ts", "getRouter().get('/x', h);\n");
        assert!(sink.routes.is_empty());
        assert!(sink.warnings[0].contains("could not tell which router"));
    }

    #[test]
    fn typescript_is_read_the_same_way() {
        let sink = extract(
            "src/app.ts",
            "import express, { Router, Request, Response } from 'express';\n\
             import { usersRouter } from './routes/users';\n\
             const app = express();\n\
             app.use('/api/users', usersRouter);\n\
             app.get('/health', (req: Request, res: Response) => res.send('ok'));\n\
             export default app;\n",
        );
        assert_eq!(sink.routers[0].symbol.name, "app");
        assert_eq!(sink.mounts[0].child.name, "usersRouter");
        assert_eq!(routes(&sink), vec!["GET /health on app"]);
        assert_eq!(sink.exports[0].exported, "default");
    }
}
