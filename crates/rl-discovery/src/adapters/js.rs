//! JavaScript and TypeScript helpers shared by the Express and Next.js adapters.
//!
//! Everything here is *recognition* — reading a literal, listing a call's arguments, turning
//! an `import` or `require` into a binding, noticing `req.query.limit` inside a handler.
//! Nothing composes a path; that is the graph's job.
//!
//! The JavaScript and TypeScript grammars share their node kinds for everything used here,
//! which is what lets one set of helpers serve `.js`, `.ts` and `.tsx` files alike.

use crate::facts::{ExportFact, ImportFact};
use crate::index::{walk, ParsedFile};
use rl_model::{AuthRequirement, PathSegment, PathTemplate};
use std::collections::BTreeMap;
use tree_sitter::Node;

/// `const NAME = "literal"` bindings, for folding a prefix that was given a name.
///
/// Deliberately shallow: literals, template strings whose substitutions fold, and `+`
/// concatenations. Anything requiring evaluation — `process.env.PREFIX`, `config.api.base`,
/// a function call — is left unresolved rather than guessed at.
#[derive(Debug, Default, Clone)]
pub struct Constants(BTreeMap<String, String>);

impl Constants {
    pub fn collect(file: &ParsedFile) -> Constants {
        let mut found = BTreeMap::new();

        // Two passes, so `B = A + "/x"` resolves when `A` is defined above it.
        for _ in 0..2 {
            walk(file.root(), &mut |node| {
                if node.kind() != "variable_declarator" {
                    return;
                }
                let (Some(name), Some(value)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("value"),
                ) else {
                    return;
                };
                if name.kind() != "identifier" {
                    return;
                }
                let scratch = Constants(found.clone());
                if let Some(text) = scratch.string_value(file, value) {
                    found.insert(file.text(name).to_string(), text);
                }
            });
        }

        Constants(found)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// Fold an expression to a string, if it can be done without evaluating anything.
    pub fn string_value(&self, file: &ParsedFile, node: Node<'_>) -> Option<String> {
        match node.kind() {
            "string" => Some(string_literal(file, node)),
            "template_string" => {
                let mut out = String::new();
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    match child.kind() {
                        "string_fragment" | "escape_sequence" => out.push_str(file.text(child)),
                        "template_substitution" => {
                            let inner = child.named_child(0)?;
                            out.push_str(&self.string_value(file, inner)?);
                        }
                        _ => {}
                    }
                }
                Some(out)
            }
            "identifier" => self.get(file.text(node)).map(str::to_string),
            "binary_expression" => {
                let operator = node.child_by_field_name("operator")?;
                if file.text(operator) != "+" {
                    return None;
                }
                let left = self.string_value(file, node.child_by_field_name("left")?)?;
                let right = self.string_value(file, node.child_by_field_name("right")?)?;
                Some(left + &right)
            }
            "parenthesized_expression" => self.string_value(file, node.named_child(0)?),
            // `"/x" as const`, `<string>"/x"`
            "as_expression" | "satisfies_expression" | "non_null_expression" => {
                self.string_value(file, node.named_child(0)?)
            }
            _ => None,
        }
    }

    /// Turn a path argument into a template, recording the source text when it cannot be
    /// worked out — a regular expression, `process.env.PREFIX`, a computed value.
    ///
    /// This is the honesty guarantee: an unresolvable prefix becomes a visible gap rather
    /// than a confidently wrong path.
    pub fn path_value(&self, file: &ParsedFile, node: Node<'_>) -> PathTemplate {
        match self.string_value(file, node) {
            Some(text) => PathTemplate::parse(&text, rl_model::ParamStyle::Colon),
            None => PathTemplate::from_segments(vec![PathSegment::unresolved(file.text(node))]),
        }
    }
}

/// The content of a plain string literal, quotes removed.
pub fn string_literal(file: &ParsedFile, node: Node<'_>) -> String {
    let mut out = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "string_fragment" | "escape_sequence") {
            out.push_str(file.text(child));
        }
    }
    out
}

/// A call's arguments, comments dropped.
pub fn arguments<'a>(call: Node<'a>) -> Vec<Node<'a>> {
    let Some(list) = call.child_by_field_name("arguments") else {
        return Vec::new();
    };
    (0..list.named_child_count() as u32)
        .filter_map(|i| list.named_child(i))
        .filter(|n| n.kind() != "comment")
        .collect()
}

/// What a call is calling: the receiver (for `a.b()`) and the method or function name.
pub fn callee<'a>(file: &ParsedFile, call: Node<'a>) -> Option<(Option<Node<'a>>, String)> {
    let function = call.child_by_field_name("function")?;
    match function.kind() {
        "identifier" => Some((None, file.text(function).to_string())),
        "member_expression" => {
            let object = function.child_by_field_name("object")?;
            let property = function.child_by_field_name("property")?;
            Some((Some(object), file.text(property).to_string()))
        }
        _ => None,
    }
}

/// The module specifier of a `require("...")` call, if that is what this node is.
pub fn require_source(file: &ParsedFile, node: Node<'_>) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    let (None, name) = callee(file, node)? else {
        return None;
    };
    if name != "require" {
        return None;
    }
    let first = arguments(node).into_iter().next()?;
    (first.kind() == "string").then(|| string_literal(file, first))
}

/// The canonical spelling of an inline `require`, used as the local name it binds.
///
/// `require('./x')` and `require("./x")` must be one name, so the quotes are normalised.
pub fn require_name(source: &str) -> String {
    format!("require(\"{source}\")")
}

/// Turn an expression into a symbol reference, if it names something.
///
/// Handles `router`, `routes.users`, `require("./routes/users")` and
/// `require("./routes").users`. An inline `require` binds nothing on its own, so one is
/// returned as an import fact for the caller to record alongside the reference.
pub fn reference(file: &ParsedFile, node: Node<'_>) -> Option<(String, Option<ImportFact>)> {
    match node.kind() {
        "identifier" => Some((file.text(node).to_string(), None)),
        "call_expression" => {
            let source = require_source(file, node)?;
            let name = require_name(&source);
            let import = ImportFact {
                module: file.path.clone(),
                local_name: name.clone(),
                source,
                original: Some("default".to_string()),
                level: 0,
            };
            Some((name, Some(import)))
        }
        "member_expression" => {
            let object = node.child_by_field_name("object")?;
            let property = node.child_by_field_name("property")?;
            let (base, import) = reference(file, object)?;
            Some((format!("{base}.{}", file.text(property)), import))
        }
        // `router as Router`, `router!`
        "as_expression" | "non_null_expression" | "parenthesized_expression" => {
            reference(file, node.named_child(0)?)
        }
        _ => None,
    }
}

/// Every name bound by an `import` statement or a `require` call in the file.
pub fn collect_imports(file: &ParsedFile) -> Vec<ImportFact> {
    let mut imports = Vec::new();
    let module = &file.path;

    let bind = |local: &str, source: &str, original: Option<&str>| ImportFact {
        module: module.clone(),
        local_name: local.to_string(),
        source: source.to_string(),
        original: original.map(str::to_string),
        level: 0,
    };

    walk(file.root(), &mut |node| match node.kind() {
        "import_statement" => {
            let Some(source) = node.child_by_field_name("source") else {
                return;
            };
            let source = string_literal(file, source);
            let Some(clause) = crate::index::child_of_kind(node, "import_clause") else {
                return; // `import "./side-effect"`
            };
            let mut cursor = clause.walk();
            for part in clause.named_children(&mut cursor) {
                match part.kind() {
                    "identifier" => {
                        imports.push(bind(file.text(part), &source, Some("default")));
                    }
                    "namespace_import" => {
                        if let Some(name) = part.named_child(0) {
                            imports.push(bind(file.text(name), &source, None));
                        }
                    }
                    "named_imports" => {
                        let mut inner = part.walk();
                        for specifier in part.named_children(&mut inner) {
                            if specifier.kind() != "import_specifier" {
                                continue;
                            }
                            let Some(name) = specifier.child_by_field_name("name") else {
                                continue;
                            };
                            let local = specifier.child_by_field_name("alias").unwrap_or(name);
                            imports.push(bind(file.text(local), &source, Some(file.text(name))));
                        }
                    }
                    _ => {}
                }
            }
        }

        // `export { a as b } from "./m"` binds nothing locally, but re-exports need a
        // name to chain through, so the exported name doubles as the local one.
        "export_statement" => {
            let (Some(source), Some(clause)) = (
                node.child_by_field_name("source"),
                crate::index::child_of_kind(node, "export_clause"),
            ) else {
                return;
            };
            let source = string_literal(file, source);
            let mut inner = clause.walk();
            for specifier in clause.named_children(&mut inner) {
                let Some(name) = specifier.child_by_field_name("name") else {
                    continue;
                };
                let exported = specifier.child_by_field_name("alias").unwrap_or(name);
                imports.push(bind(file.text(exported), &source, Some(file.text(name))));
            }
        }

        "variable_declarator" => {
            let (Some(name), Some(value)) = (
                node.child_by_field_name("name"),
                node.child_by_field_name("value"),
            ) else {
                return;
            };

            // `const x = require("./m")` and `const x = require("./m").y`
            let (source, member) = match require_source(file, value) {
                Some(source) => (source, None),
                None if value.kind() == "member_expression" => {
                    let (Some(object), Some(property)) = (
                        value.child_by_field_name("object"),
                        value.child_by_field_name("property"),
                    ) else {
                        return;
                    };
                    match require_source(file, object) {
                        Some(source) => (source, Some(file.text(property).to_string())),
                        None => return,
                    }
                }
                None => return,
            };

            match name.kind() {
                "identifier" => {
                    let original = member.as_deref().unwrap_or("default");
                    imports.push(bind(file.text(name), &source, Some(original)));
                }
                // `const { a, b: c } = require("./m")`
                "object_pattern" if member.is_none() => {
                    let mut inner = name.walk();
                    for entry in name.named_children(&mut inner) {
                        match entry.kind() {
                            "shorthand_property_identifier_pattern" => {
                                imports.push(bind(
                                    file.text(entry),
                                    &source,
                                    Some(file.text(entry)),
                                ));
                            }
                            "pair_pattern" => {
                                if let (Some(key), Some(value)) = (
                                    entry.child_by_field_name("key"),
                                    entry.child_by_field_name("value"),
                                ) {
                                    if value.kind() == "identifier" {
                                        imports.push(bind(
                                            file.text(value),
                                            &source,
                                            Some(file.text(key)),
                                        ));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    });

    imports
}

/// Every name the file exports, in both module systems.
///
/// `module.exports = router` — the CommonJS default — is the one that matters most, since
/// it is how nearly every Express router file ends.
pub fn collect_exports(file: &ParsedFile) -> Vec<ExportFact> {
    let mut exports = Vec::new();
    let module = &file.path;

    let mut export = |exported: &str, local: &str| {
        exports.push(ExportFact {
            module: module.clone(),
            exported: exported.to_string(),
            local: local.to_string(),
        });
    };

    walk(file.root(), &mut |node| match node.kind() {
        "export_statement" => {
            if node.child_by_field_name("source").is_some() {
                // Re-export: recorded as an import under the exported name.
                if let Some(clause) = crate::index::child_of_kind(node, "export_clause") {
                    let mut inner = clause.walk();
                    for specifier in clause.named_children(&mut inner) {
                        let Some(name) = specifier.child_by_field_name("name") else {
                            continue;
                        };
                        let exported = specifier.child_by_field_name("alias").unwrap_or(name);
                        export(file.text(exported), file.text(exported));
                    }
                }
                return;
            }

            let is_default = (0..node.child_count() as u32)
                .filter_map(|i| node.child(i))
                .any(|c| c.kind() == "default");

            // `export default router`
            if let Some(value) = node.child_by_field_name("value") {
                if value.kind() == "identifier" {
                    export("default", file.text(value));
                }
                return;
            }

            // `export { a, b as c }`
            if let Some(clause) = crate::index::child_of_kind(node, "export_clause") {
                let mut inner = clause.walk();
                for specifier in clause.named_children(&mut inner) {
                    let Some(name) = specifier.child_by_field_name("name") else {
                        continue;
                    };
                    let exported = specifier.child_by_field_name("alias").unwrap_or(name);
                    export(file.text(exported), file.text(name));
                }
                return;
            }

            // `export const x = ...`, `export function x`, `export default function x`
            if let Some(declaration) = node.child_by_field_name("declaration") {
                for name in declared_names(file, declaration) {
                    export(&name, &name);
                    if is_default {
                        export("default", &name);
                    }
                }
            }
        }

        "assignment_expression" => {
            let (Some(left), Some(right)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) else {
                return;
            };
            let target = file.text(left);
            let exported = if target == "module.exports" {
                "default"
            } else if let Some(name) = target
                .strip_prefix("module.exports.")
                .or_else(|| target.strip_prefix("exports."))
            {
                name
            } else {
                return;
            };

            match right.kind() {
                "identifier" => export(exported, file.text(right)),
                // `module.exports = { users, admin: adminRouter }`
                "object" if exported == "default" => {
                    let mut inner = right.walk();
                    for entry in right.named_children(&mut inner) {
                        match entry.kind() {
                            "shorthand_property_identifier" => {
                                export(file.text(entry), file.text(entry));
                            }
                            "pair" => {
                                if let (Some(key), Some(value)) = (
                                    entry.child_by_field_name("key"),
                                    entry.child_by_field_name("value"),
                                ) {
                                    if value.kind() == "identifier" {
                                        export(
                                            file.text(key).trim_matches(['"', '\'']),
                                            file.text(value),
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    });

    exports
}

/// The names a declaration introduces: `const a = 1, b = 2` → `a`, `b`.
pub fn declared_names(file: &ParsedFile, declaration: Node<'_>) -> Vec<String> {
    let mut names = Vec::new();
    match declaration.kind() {
        "function_declaration" | "class_declaration" | "generator_function_declaration" => {
            if let Some(name) = declaration.child_by_field_name("name") {
                names.push(file.text(name).to_string());
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            let mut cursor = declaration.walk();
            for declarator in declaration.named_children(&mut cursor) {
                let Some(name) = declarator.child_by_field_name("name") else {
                    continue;
                };
                match name.kind() {
                    "identifier" => names.push(file.text(name).to_string()),
                    // `export const { GET, POST } = handlers`
                    "object_pattern" => {
                        let mut inner = name.walk();
                        for entry in name.named_children(&mut inner) {
                            match entry.kind() {
                                "shorthand_property_identifier_pattern" => {
                                    names.push(file.text(entry).to_string());
                                }
                                "pair_pattern" => {
                                    if let Some(value) = entry.child_by_field_name("value") {
                                        if value.kind() == "identifier" {
                                            names.push(file.text(value).to_string());
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    names
}

/// What a handler reads off the request object: the closest thing Express has to a
/// signature.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RequestUsage {
    /// `req.query.limit`, `const { q } = req.query`
    pub query: Vec<String>,
    /// `req.body.email`
    pub body: Vec<String>,
    /// `req.headers["x-api-key"]`, `req.get("Authorization")`
    pub headers: Vec<String>,
    /// `req.body` was touched at all, even without a named field.
    pub reads_body: bool,
}

impl RequestUsage {
    /// Inspect a handler function for what it reads from its request parameter.
    ///
    /// The parameter is whatever the function names first — `req`, `request`, `r` — so
    /// nothing depends on a naming convention.
    pub fn of(file: &ParsedFile, function: Node<'_>) -> RequestUsage {
        let mut usage = RequestUsage::default();

        let Some(request) = first_parameter_name(file, function) else {
            return usage;
        };
        let Some(body) = function.child_by_field_name("body") else {
            return usage;
        };

        walk(body, &mut |node| match node.kind() {
            "member_expression" => {
                let (Some(object), Some(property)) = (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("property"),
                ) else {
                    return;
                };
                if object.kind() != "member_expression" {
                    return;
                }
                let (Some(base), Some(field)) = (
                    object.child_by_field_name("object"),
                    object.child_by_field_name("property"),
                ) else {
                    return;
                };
                if file.text(base) != request {
                    return;
                }
                let name = file.text(property).to_string();
                match file.text(field) {
                    "query" => push_unique(&mut usage.query, name),
                    "body" => {
                        usage.reads_body = true;
                        push_unique(&mut usage.body, name);
                    }
                    "headers" => push_unique(&mut usage.headers, name),
                    _ => {}
                }
            }
            "subscript_expression" => {
                let (Some(object), Some(index)) = (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("index"),
                ) else {
                    return;
                };
                if index.kind() != "string" {
                    return;
                }
                let Some((base, field)) = split_member(file, object) else {
                    return;
                };
                if base != request {
                    return;
                }
                let name = string_literal(file, index);
                match field.as_str() {
                    "query" => push_unique(&mut usage.query, name),
                    "body" => {
                        usage.reads_body = true;
                        push_unique(&mut usage.body, name);
                    }
                    "headers" => push_unique(&mut usage.headers, name),
                    _ => {}
                }
            }
            "call_expression" => {
                // `req.get("X-Header")`, `req.header("X-Header")`
                let Some((Some(receiver), method)) = callee(file, node) else {
                    return;
                };
                if file.text(receiver) != request || !matches!(method.as_str(), "get" | "header") {
                    return;
                }
                if let Some(first) = arguments(node).into_iter().next() {
                    if first.kind() == "string" {
                        push_unique(&mut usage.headers, string_literal(file, first));
                    }
                }
            }
            "variable_declarator" => {
                // `const { limit, offset } = req.query`
                let (Some(name), Some(value)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("value"),
                ) else {
                    return;
                };
                if name.kind() != "object_pattern" {
                    return;
                }
                let Some((base, field)) = split_member(file, value) else {
                    return;
                };
                if base != request {
                    return;
                }
                let target = match field.as_str() {
                    "query" => &mut usage.query,
                    "body" => {
                        usage.reads_body = true;
                        &mut usage.body
                    }
                    "headers" => &mut usage.headers,
                    _ => return,
                };
                let mut inner = name.walk();
                for entry in name.named_children(&mut inner) {
                    let key = match entry.kind() {
                        "shorthand_property_identifier_pattern" => Some(entry),
                        "pair_pattern" | "object_assignment_pattern" => {
                            // `{ limit = 20 }` is an object_assignment_pattern whose
                            // left side is the shorthand; `{ q: query }` is a pair.
                            entry
                                .child_by_field_name("key")
                                .or_else(|| entry.child_by_field_name("left"))
                        }
                        _ => None,
                    };
                    if let Some(key) = key {
                        push_unique(target, file.text(key).to_string());
                    }
                }
            }
            _ => {}
        });

        // `req.body` read whole, with no field named — still a body.
        if !usage.reads_body {
            walk(body, &mut |node| {
                if node.kind() == "member_expression" {
                    if let Some((base, field)) = split_member(file, node) {
                        if base == request && field == "body" {
                            usage.reads_body = true;
                        }
                    }
                }
            });
        }

        usage
    }
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// `a.b` → (`"a"`, `"b"`), for a member expression whose object is a plain identifier.
fn split_member(file: &ParsedFile, node: Node<'_>) -> Option<(String, String)> {
    if node.kind() != "member_expression" {
        return None;
    }
    let object = node.child_by_field_name("object")?;
    let property = node.child_by_field_name("property")?;
    if object.kind() != "identifier" {
        return None;
    }
    Some((
        file.text(object).to_string(),
        file.text(property).to_string(),
    ))
}

/// The name of a function's first parameter, in JavaScript or TypeScript form.
pub fn first_parameter_name(file: &ParsedFile, function: Node<'_>) -> Option<String> {
    let parameters = function.child_by_field_name("parameters")?;
    let first = parameters.named_child(0)?;
    let pattern = match first.kind() {
        // TypeScript wraps each parameter: `(req: Request)`
        "required_parameter" | "optional_parameter" => first.child_by_field_name("pattern")?,
        _ => first,
    };
    (pattern.kind() == "identifier").then(|| file.text(pattern).to_string())
}

/// Whether a node is a function of any spelling.
pub fn is_function(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "arrow_function" | "function_expression" | "function_declaration" | "function"
    )
}

/// A top-level function or function-valued constant by name, so a handler passed by
/// reference can still be inspected.
pub fn top_level_function<'a>(file: &'a ParsedFile, name: &str) -> Option<Node<'a>> {
    let root = file.root();
    let mut found = None;
    walk(root, &mut |node| {
        if found.is_some() {
            return;
        }
        match node.kind() {
            "function_declaration" => {
                if node
                    .child_by_field_name("name")
                    .is_some_and(|n| file.text(n) == name)
                {
                    found = Some(node);
                }
            }
            "variable_declarator" => {
                let (Some(id), Some(value)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("value"),
                ) else {
                    return;
                };
                if file.text(id) == name && is_function(value) {
                    found = Some(value);
                }
            }
            _ => {}
        }
    });
    found
}

/// A handler that reads the `Authorization` header is enforcing something with it.
///
/// Turning the header into an auth requirement is more useful than listing it as a header
/// the caller has to type: the request editor can then offer the right tab.
pub fn promote_authorization_header(
    headers: &mut Vec<rl_model::ParamSpec>,
    auth: &mut Option<AuthRequirement>,
) {
    let position = headers
        .iter()
        .position(|h| h.name.eq_ignore_ascii_case("authorization"));
    if let Some(index) = position {
        headers.remove(index);
        if auth.is_none() {
            *auth = Some(AuthRequirement::Unknown {
                hint: "reads the Authorization header".to_string(),
            });
        }
    }
}

/// Names that suggest a middleware enforces authentication.
const AUTH_HINTS: &[&str] = &[
    "auth",
    "protect",
    "guard",
    "jwt",
    "token",
    "verify",
    "loggedin",
    "logged_in",
    "ensure",
    "session",
    "passport",
    "requireuser",
    "require_user",
    "permission",
];

/// Guess at authentication from a middleware's name.
///
/// A heuristic, and reported as one: the result is `Unknown` with the name as the hint
/// unless the name itself says which scheme. A middleware called `validate` is not counted,
/// and one called `authLogger` is — both are wrong, and the UI shows the name so the
/// developer can tell.
pub fn auth_from_middleware(name: &str) -> Option<AuthRequirement> {
    let lowered = name.to_ascii_lowercase();
    // `jwtRouter` and `auth.routes` are routers that happen to be *about* auth.
    if ["router", "routes", "route"]
        .iter()
        .any(|s| lowered.ends_with(s))
    {
        return None;
    }
    if !AUTH_HINTS.iter().any(|hint| lowered.contains(hint)) {
        return None;
    }
    if lowered.contains("jwt") || lowered.contains("bearer") {
        return Some(AuthRequirement::Bearer {
            format: lowered.contains("jwt").then(|| "JWT".to_string()),
        });
    }
    if lowered.contains("basic") {
        return Some(AuthRequirement::Basic);
    }
    Some(AuthRequirement::Unknown {
        hint: name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::SourceIndex;
    use crate::project::Language;

    fn parse(path: &str, source: &str) -> ParsedFile {
        let language = Language::of(std::path::Path::new(path)).unwrap();
        SourceIndex::new()
            .unwrap()
            .parse(path, language, source.to_string())
            .unwrap()
    }

    fn imports(path: &str, source: &str) -> Vec<(String, String, Option<String>)> {
        collect_imports(&parse(path, source))
            .into_iter()
            .map(|i| (i.local_name, i.source, i.original))
            .collect()
    }

    fn exports(path: &str, source: &str) -> Vec<(String, String)> {
        collect_exports(&parse(path, source))
            .into_iter()
            .map(|e| (e.exported, e.local))
            .collect()
    }

    fn s(text: &str) -> String {
        text.to_string()
    }

    #[test]
    fn esm_imports_bind_default_named_aliased_and_namespace() {
        let found = imports(
            "app.ts",
            "import express, { Router, json as parseJson } from 'express';\n\
             import * as routes from './routes';\n\
             import users from './routes/users';\n",
        );
        assert_eq!(
            found,
            vec![
                (s("express"), s("express"), Some(s("default"))),
                (s("Router"), s("express"), Some(s("Router"))),
                (s("parseJson"), s("express"), Some(s("json"))),
                (s("routes"), s("./routes"), None),
                (s("users"), s("./routes/users"), Some(s("default"))),
            ]
        );
    }

    #[test]
    fn commonjs_requires_bind_default_destructured_and_member() {
        let found = imports(
            "app.js",
            "const express = require('express');\n\
             const { Router, json: parseJson } = require('express');\n\
             const items = require('./routes/items').router;\n",
        );
        assert_eq!(
            found,
            vec![
                (s("express"), s("express"), Some(s("default"))),
                (s("Router"), s("express"), Some(s("Router"))),
                (s("parseJson"), s("express"), Some(s("json"))),
                (s("items"), s("./routes/items"), Some(s("router"))),
            ]
        );
    }

    #[test]
    fn a_re_export_is_an_import_under_the_exported_name() {
        let file = parse("index.ts", "export { router as users } from './users';\n");
        let imports = collect_imports(&file);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].local_name, "users");
        assert_eq!(imports[0].original.as_deref(), Some("router"));

        let exports = collect_exports(&file);
        assert_eq!(exports[0].exported, "users");
        assert_eq!(exports[0].local, "users");
    }

    #[test]
    fn commonjs_exports_in_every_spelling() {
        let found = exports(
            "users.js",
            "module.exports = router;\n\
             module.exports = { users, admin: adminRouter };\n\
             exports.legacy = legacyRouter;\n\
             module.exports.extra = extraRouter;\n",
        );
        assert_eq!(
            found,
            vec![
                (s("default"), s("router")),
                (s("users"), s("users")),
                (s("admin"), s("adminRouter")),
                (s("legacy"), s("legacyRouter")),
                (s("extra"), s("extraRouter")),
            ]
        );
    }

    #[test]
    fn esm_exports_in_every_spelling() {
        let found = exports(
            "users.ts",
            "export const router = Router();\n\
             export default router;\n\
             export { router as usersRouter, other };\n\
             export async function GET() {}\n\
             export default function handler() {}\n\
             export const { PUT, PATCH } = handlers;\n",
        );
        assert_eq!(
            found,
            vec![
                (s("router"), s("router")),
                (s("default"), s("router")),
                (s("usersRouter"), s("router")),
                (s("other"), s("other")),
                (s("GET"), s("GET")),
                (s("handler"), s("handler")),
                (s("default"), s("handler")),
                (s("PUT"), s("PUT")),
                (s("PATCH"), s("PATCH")),
            ]
        );
    }

    #[test]
    fn constants_fold_literals_templates_and_concatenation() {
        let file = parse(
            "a.js",
            "const BASE = '/api';\n\
             const V1 = `${BASE}/v1`;\n\
             const USERS = V1 + '/users';\n\
             const ENV = process.env.PREFIX;\n",
        );
        let constants = Constants::collect(&file);
        assert_eq!(constants.get("BASE"), Some("/api"));
        assert_eq!(constants.get("V1"), Some("/api/v1"));
        assert_eq!(constants.get("USERS"), Some("/api/v1/users"));
        assert_eq!(constants.get("ENV"), None, "environment reads never fold");
    }

    #[test]
    fn an_unfoldable_path_becomes_a_visible_gap() {
        let file = parse(
            "a.js",
            "app.use(process.env.PREFIX, r);\napp.get(/\\/ab?c/, h);\n",
        );
        let constants = Constants::collect(&file);

        let mut paths = Vec::new();
        walk(file.root(), &mut |node| {
            if node.kind() == "call_expression" {
                if let Some(first) = arguments(node).into_iter().next() {
                    paths.push(constants.path_value(&file, first));
                }
            }
        });

        assert_eq!(paths.len(), 2);
        assert!(!paths[0].is_resolved());
        assert_eq!(paths[0].unresolved_exprs(), vec!["process.env.PREFIX"]);
        assert_eq!(paths[1].unresolved_exprs(), vec!["/\\/ab?c/"]);
    }

    #[test]
    fn references_cover_identifiers_members_and_inline_requires() {
        let file = parse(
            "a.js",
            "f(router);\nf(routes.users);\nf(require('./routes/users'));\nf(require(\"./r\").x);\n",
        );
        let mut found = Vec::new();
        walk(file.root(), &mut |node| {
            if node.kind() == "call_expression" && callee(&file, node).unwrap().1 == "f" {
                let (name, import) = reference(&file, arguments(node)[0]).unwrap();
                found.push((name, import.map(|i| i.source)));
            }
        });
        assert_eq!(
            found,
            vec![
                (s("router"), None),
                (s("routes.users"), None),
                (s("require(\"./routes/users\")"), Some(s("./routes/users"))),
                (s("require(\"./r\").x"), Some(s("./r"))),
            ]
        );
    }

    #[test]
    fn request_usage_is_read_off_the_handler_body() {
        let file = parse(
            "a.js",
            "app.post('/x', (request, res) => {\n\
               const { limit, offset = 0 } = request.query;\n\
               const page = request.query.page;\n\
               const email = request.body.email;\n\
               const key = request.headers['x-api-key'];\n\
               const ua = request.get('User-Agent');\n\
             });\n",
        );
        let mut usage = None;
        walk(file.root(), &mut |node| {
            if node.kind() == "arrow_function" {
                usage = Some(RequestUsage::of(&file, node));
            }
        });
        let usage = usage.unwrap();
        assert_eq!(usage.query, vec!["limit", "offset", "page"]);
        assert_eq!(usage.body, vec!["email"]);
        assert_eq!(usage.headers, vec!["x-api-key", "User-Agent"]);
        assert!(usage.reads_body);
    }

    #[test]
    fn a_typescript_handler_parameter_is_found_through_its_annotation() {
        let file = parse(
            "a.ts",
            "app.get('/x', (req: Request, res: Response) => { const q = req.query.q; });\n",
        );
        let mut usage = None;
        walk(file.root(), &mut |node| {
            if node.kind() == "arrow_function" {
                usage = Some(RequestUsage::of(&file, node));
            }
        });
        assert_eq!(usage.unwrap().query, vec!["q"]);
    }

    #[test]
    fn a_handler_passed_by_name_is_found_at_the_top_level() {
        let file = parse(
            "a.js",
            "function list(req, res) { req.query.limit; }\n\
             const create = async (req, res) => { req.body.name; };\n\
             app.get('/', list);\napp.post('/', create);\n",
        );
        let list = top_level_function(&file, "list").unwrap();
        assert_eq!(RequestUsage::of(&file, list).query, vec!["limit"]);
        let create = top_level_function(&file, "create").unwrap();
        assert_eq!(RequestUsage::of(&file, create).body, vec!["name"]);
        assert!(top_level_function(&file, "missing").is_none());
    }

    #[test]
    fn reading_the_authorization_header_counts_as_auth() {
        let mut headers = vec![
            rl_model::ParamSpec::new("X-Trace-Id"),
            rl_model::ParamSpec::new("authorization"),
        ];
        let mut auth = None;
        promote_authorization_header(&mut headers, &mut auth);
        assert_eq!(headers.len(), 1);
        assert!(matches!(auth, Some(AuthRequirement::Unknown { .. })));

        // An auth requirement already known from a middleware name is not overwritten.
        let mut headers = vec![rl_model::ParamSpec::new("Authorization")];
        let mut auth = Some(AuthRequirement::Basic);
        promote_authorization_header(&mut headers, &mut auth);
        assert_eq!(auth, Some(AuthRequirement::Basic));
    }

    #[test]
    fn auth_is_guessed_from_middleware_names_and_says_so() {
        assert!(matches!(
            auth_from_middleware("requireAuth"),
            Some(AuthRequirement::Unknown { hint }) if hint == "requireAuth"
        ));
        assert!(matches!(
            auth_from_middleware("verifyJwt"),
            Some(AuthRequirement::Bearer { format: Some(f) }) if f == "JWT"
        ));
        assert_eq!(auth_from_middleware("validateBody"), None);
        assert_eq!(auth_from_middleware("upload.single"), None);
        assert_eq!(
            auth_from_middleware("authRouter"),
            None,
            "a router about auth"
        );
        assert_eq!(auth_from_middleware("sessionRoutes"), None);
    }
}
