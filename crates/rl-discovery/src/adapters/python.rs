//! Python helpers shared by the FastAPI and Flask adapters.
//!
//! Everything here is *recognition* — reading a literal, finding a keyword argument, folding a
//! constant. Nothing composes a path; that is the graph's job.

use crate::facts::{ImportFact, RouteFact};
use crate::index::{walk, ParsedFile};
use rl_model::{AuthRequirement, BodySchema, ParamSpec, ParamStyle, PathSegment, PathTemplate};
use std::collections::BTreeMap;
use tree_sitter::Node;

/// Module-level `NAME = "literal"` bindings, for folding a prefix that was given a name.
///
/// Deliberately shallow: literals, and concatenations of literals. Anything requiring
/// evaluation — `settings.API_PREFIX`, `os.environ[...]`, a function call — is left
/// unresolved rather than guessed at.
#[derive(Debug, Default, Clone)]
pub struct Constants(BTreeMap<String, String>);

impl Constants {
    /// Collect from a file's top level.
    pub fn collect(file: &ParsedFile) -> Constants {
        let mut found = BTreeMap::new();

        // Two passes, so `B = A + "/x"` resolves when `A` is defined above it.
        for _ in 0..2 {
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
                if left.kind() != "identifier" {
                    return;
                }
                let scratch = Constants(found.clone());
                if let Some(value) = scratch.string_value(file, right) {
                    found.insert(file.text(left).to_string(), value);
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
            "string" => self.string_literal(file, node),
            "identifier" => self.get(file.text(node)).map(str::to_string),
            "concatenated_string" => {
                let mut out = String::new();
                let mut cursor = node.walk();
                for part in node.children(&mut cursor) {
                    out.push_str(&self.string_value(file, part)?);
                }
                Some(out)
            }
            "binary_operator" => {
                // Only `+`. Anything else on strings is not concatenation.
                let operator = node.child(1)?;
                if file.text(operator) != "+" {
                    return None;
                }
                let left = self.string_value(file, node.child_by_field_name("left")?)?;
                let right = self.string_value(file, node.child_by_field_name("right")?)?;
                Some(left + &right)
            }
            "parenthesized_expression" => {
                let inner = node.named_child(0)?;
                self.string_value(file, inner)
            }
            _ => None,
        }
    }

    /// A string literal, including an f-string whose interpolations are all foldable.
    fn string_literal(&self, file: &ParsedFile, node: Node<'_>) -> Option<String> {
        let mut out = String::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                "string_content" => out.push_str(file.text(child)),
                "interpolation" => {
                    // f"{PREFIX}/users" is foldable only when PREFIX is.
                    let expression = child.named_child(0)?;
                    out.push_str(&self.string_value(file, expression)?);
                }
                "escape_sequence" => out.push_str(file.text(child)),
                _ => {}
            }
        }

        Some(out)
    }

    /// Turn a path argument into a template, recording the source text when it cannot be
    /// worked out.
    ///
    /// This is the honesty guarantee: an unresolvable prefix becomes a visible gap rather
    /// than a confidently wrong path.
    pub fn path_value(&self, file: &ParsedFile, node: Node<'_>) -> PathTemplate {
        self.path_value_in(file, node, ParamStyle::Braces)
    }

    /// [`Constants::path_value`] for a framework with its own parameter syntax — Flask's
    /// `<int:user_id>`.
    pub fn path_value_in(
        &self,
        file: &ParsedFile,
        node: Node<'_>,
        style: ParamStyle,
    ) -> PathTemplate {
        match self.string_value(file, node) {
            Some(text) => {
                let mut template = PathTemplate::parse(&text, style);
                // `@router.get("/")` under `APIRouter(prefix="/users")` serves `/users/`,
                // and Starlette answers `/users` with a 307 to it. Werkzeug does the same
                // for Blueprints. So a bare `/` keeps its slash when joined onto a prefix;
                // on the app root it still renders as `/`.
                if text.trim() == "/" {
                    template.trailing_slash = true;
                }
                template
            }
            None => PathTemplate::from_segments(vec![PathSegment::unresolved(file.text(node))]),
        }
    }
}

/// Positional and keyword arguments of a call.
pub struct Arguments<'a> {
    pub positional: Vec<Node<'a>>,
    pub keyword: BTreeMap<String, Node<'a>>,
}

impl<'a> Arguments<'a> {
    pub fn of(file: &ParsedFile, call: Node<'a>) -> Arguments<'a> {
        let mut positional = Vec::new();
        let mut keyword = BTreeMap::new();

        if let Some(list) = call.child_by_field_name("arguments") {
            let mut cursor = list.walk();
            for argument in list.named_children(&mut cursor) {
                if argument.kind() == "keyword_argument" {
                    if let (Some(name), Some(value)) = (
                        argument.child_by_field_name("name"),
                        argument.child_by_field_name("value"),
                    ) {
                        keyword.insert(file.text(name).to_string(), value);
                    }
                } else if argument.kind() != "comment" {
                    positional.push(argument);
                }
            }
        }

        Arguments {
            positional,
            keyword,
        }
    }

    pub fn first_positional(&self) -> Option<Node<'a>> {
        self.positional.first().copied()
    }

    pub fn keyword(&self, name: &str) -> Option<Node<'a>> {
        self.keyword.get(name).copied()
    }
}

/// The first string in a list literal — how `tags=["users"]` becomes a group.
pub fn first_list_string(
    file: &ParsedFile,
    node: Node<'_>,
    constants: &Constants,
) -> Option<String> {
    if node.kind() != "list" && node.kind() != "tuple" {
        return constants.string_value(file, node);
    }
    (0..node.named_child_count() as u32)
        .filter_map(|i| node.named_child(i))
        .find_map(|item| constants.string_value(file, item))
}

/// Every string in a list literal — how `methods=["GET", "POST"]` becomes methods.
pub fn list_strings(file: &ParsedFile, node: Node<'_>, constants: &Constants) -> Vec<String> {
    if node.kind() != "list" && node.kind() != "tuple" {
        return constants.string_value(file, node).into_iter().collect();
    }
    (0..node.named_child_count() as u32)
        .filter_map(|i| node.named_child(i))
        .filter_map(|item| constants.string_value(file, item))
        .collect()
}

/// The name of the callee: `router.get` → `("router", "get")`, `FastAPI` → `(None, "FastAPI")`.
pub fn callee(file: &ParsedFile, call: Node<'_>) -> Option<(Option<String>, String)> {
    let function = call.child_by_field_name("function")?;
    match function.kind() {
        "identifier" => Some((None, file.text(function).to_string())),
        "attribute" => {
            let object = function.child_by_field_name("object")?;
            let attribute = function.child_by_field_name("attribute")?;
            Some((
                Some(file.text(object).to_string()),
                file.text(attribute).to_string(),
            ))
        }
        _ => None,
    }
}

/// The name of the function a node sits inside, if any — `create_app` for an app factory.
pub fn enclosing_function(file: &ParsedFile, node: Node<'_>) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "function_definition" {
            return parent
                .child_by_field_name("name")
                .map(|n| file.text(n).to_string());
        }
        current = parent.parent();
    }
    None
}

/// The handler's docstring, used as a summary when the decorator gives none.
pub fn docstring(file: &ParsedFile, definition: Node<'_>) -> Option<String> {
    let body = definition.child_by_field_name("body")?;
    let first = body.named_child(0)?;
    let expression = if first.kind() == "expression_statement" {
        first.named_child(0)?
    } else {
        first
    };
    if expression.kind() != "string" {
        return None;
    }

    let text = file.text(expression);
    let trimmed = text
        .trim_start_matches(['r', 'b', 'f', 'R', 'B', 'F'])
        .trim_matches(|c| c == '"' || c == '\'');
    trimmed
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// Guess an auth scheme from the name of a dependency or decorator.
///
/// A heuristic, and labelled as one: only names that clearly indicate authentication
/// count (`Depends(get_db)` is a database session, not a security scheme), and anything
/// recognised as auth but not as a scheme becomes [`AuthRequirement::Unknown`] carrying
/// the name rather than a confident claim.
pub fn auth_from_name(hint: &str) -> Option<AuthRequirement> {
    let lowered = hint.to_ascii_lowercase();

    if lowered.contains("oauth") || lowered.contains("bearer") || lowered.contains("jwt") {
        return Some(AuthRequirement::Bearer { format: None });
    }
    if lowered.contains("basic") {
        return Some(AuthRequirement::Basic);
    }
    if lowered.contains("api_key") || lowered.contains("apikey") {
        return Some(AuthRequirement::ApiKey {
            name: hint.to_string(),
            location: rl_model::ApiKeyLocation::Header,
        });
    }
    // `get_current_user`, `get_current_admin`, `require_role`, `logged_in_user`,
    // `login_required`, `token_required`.
    if lowered.contains("auth")
        || lowered.contains("current_")
        || lowered.contains("require")
        || lowered.contains("logged")
        || lowered.contains("login")
        || lowered.contains("token")
        || lowered.contains("security")
        || lowered.contains("permission")
    {
        return Some(AuthRequirement::Unknown {
            hint: hint.to_string(),
        });
    }

    None
}

/// Collect every import in a file.
///
/// The router declared in `api/users.py` and the `include_router` call in `main.py` are
/// connected only by an import statement, so this is what makes cross-file resolution
/// possible at all.
pub fn collect_imports(file: &ParsedFile) -> Vec<ImportFact> {
    let mut imports = Vec::new();
    let module = file.path.clone();

    walk(file.root(), &mut |node| match node.kind() {
        "import_from_statement" => {
            let Some(source_node) = node.child_by_field_name("module_name") else {
                return;
            };

            let (source, level) = match source_node.kind() {
                "relative_import" => {
                    let text = file.text(source_node);
                    let dots = text.chars().take_while(|c| *c == '.').count() as u32;
                    (text.trim_start_matches('.').to_string(), dots)
                }
                _ => (file.text(source_node).to_string(), 0),
            };

            let mut cursor = node.walk();
            for name_node in node.children_by_field_name("name", &mut cursor) {
                match name_node.kind() {
                    "aliased_import" => {
                        let (Some(name), Some(alias)) = (
                            name_node.child_by_field_name("name"),
                            name_node.child_by_field_name("alias"),
                        ) else {
                            continue;
                        };
                        imports.push(ImportFact {
                            module: module.clone(),
                            local_name: file.text(alias).to_string(),
                            source: source.clone(),
                            original: Some(file.text(name).to_string()),
                            level,
                        });
                    }
                    "dotted_name" | "identifier" => {
                        let name = file.text(name_node).to_string();
                        imports.push(ImportFact {
                            module: module.clone(),
                            local_name: name.clone(),
                            source: source.clone(),
                            original: Some(name),
                            level,
                        });
                    }
                    _ => {}
                }
            }
        }

        "import_statement" => {
            let mut cursor = node.walk();
            for name_node in node.children_by_field_name("name", &mut cursor) {
                match name_node.kind() {
                    "aliased_import" => {
                        let (Some(name), Some(alias)) = (
                            name_node.child_by_field_name("name"),
                            name_node.child_by_field_name("alias"),
                        ) else {
                            continue;
                        };
                        imports.push(ImportFact {
                            module: module.clone(),
                            local_name: file.text(alias).to_string(),
                            source: file.text(name).to_string(),
                            original: None,
                            level: 0,
                        });
                    }
                    "dotted_name" => {
                        // `import api.users` binds the whole dotted path as the local name.
                        let name = file.text(name_node).to_string();
                        imports.push(ImportFact {
                            module: module.clone(),
                            local_name: name.clone(),
                            source: name,
                            original: None,
                            level: 0,
                        });
                    }
                    _ => {}
                }
            }
        }

        _ => {}
    });

    imports
}

/// How a framework spells the parts of its request object, so [`RequestUsage`] can read
/// `request.args.get("q")` and `request.GET.get("q")` with one walker.
pub struct RequestDialect {
    /// Receivers whose `.get(...)` / `[...]` name a query parameter.
    pub query: &'static [&'static str],
    /// Receivers whose `.get(...)` / `[...]` name a header, as spelled on the wire.
    pub headers: &'static [&'static str],
    /// Receivers whose keys are WSGI-style — Django's `request.META["HTTP_X_TOKEN"]`.
    pub meta_headers: &'static [&'static str],
    /// Expressions that read a body, with the content type each implies.
    pub bodies: &'static [(&'static str, &'static str)],
}

impl RequestDialect {
    fn body_content_type(&self, text: &str) -> Option<&'static str> {
        self.bodies
            .iter()
            .find(|(expr, _)| *expr == text)
            .map(|(_, ty)| *ty)
    }
}

/// What a handler reads from its request object: query parameters, headers, and a body
/// with the keys taken out of it.
///
/// Best-effort by design. Where a framework declares these in a signature (FastAPI) the
/// signature is the better source; Flask and Django only ever say so in the body.
#[derive(Debug, Default)]
pub struct RequestUsage {
    pub query: Vec<ParamSpec>,
    pub headers: Vec<ParamSpec>,
    pub body: Option<BodySchema>,
    pub body_fields: Vec<String>,
}

impl RequestUsage {
    pub fn of(
        file: &ParsedFile,
        definition: Node<'_>,
        path_names: &[&str],
        dialect: &RequestDialect,
    ) -> RequestUsage {
        let mut usage = RequestUsage::default();
        let Some(body) = definition.child_by_field_name("body") else {
            return usage;
        };

        // `data = request.get_json()` / `form = request.form` — names the body travels under.
        let mut body_aliases: Vec<String> = Vec::new();
        walk(body, &mut |node| {
            if node.kind() != "assignment" {
                return;
            }
            let (Some(left), Some(right)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) else {
                return;
            };
            if left.kind() == "identifier" && body_source(file, right, dialect).is_some() {
                body_aliases.push(file.text(left).to_string());
            }
        });

        walk(body, &mut |node| match node.kind() {
            "call" => {
                let Some(function) = node.child_by_field_name("function") else {
                    return;
                };
                if function.kind() != "attribute" {
                    return;
                }
                let (Some(object), Some(attribute)) = (
                    function.child_by_field_name("object"),
                    function.child_by_field_name("attribute"),
                ) else {
                    return;
                };
                let receiver = file.text(object);
                let method = file.text(attribute);
                let args = Arguments::of(file, node);
                let key = args
                    .first_positional()
                    .and_then(|n| Constants::default().string_value(file, n));

                if let Some(content_type) = body_source(file, node, dialect) {
                    usage.body.get_or_insert_with(|| BodySchema {
                        content_type: content_type.to_string(),
                        schema: None,
                        example: None,
                        required: false,
                    });
                    return;
                }

                let Some(key) = key else { return };
                if !matches!(method, "get" | "getlist" | "pop") {
                    return;
                }
                if dialect.query.contains(&receiver) {
                    let mut spec = ParamSpec::new(&key);
                    spec.default = args
                        .positional
                        .get(1)
                        .copied()
                        .or_else(|| args.keyword("default"))
                        .and_then(|n| literal(file, n));
                    usage.push_query(spec, path_names);
                } else if dialect.headers.contains(&receiver) {
                    usage.push_header(&key);
                } else if dialect.meta_headers.contains(&receiver) {
                    if let Some(name) = meta_header_name(&key) {
                        usage.push_header(&name);
                    }
                } else if let Some(content_type) = dialect.body_content_type(receiver) {
                    // `request.form.get("name")` — a keyed read of a body.
                    usage.body.get_or_insert_with(|| BodySchema {
                        content_type: content_type.to_string(),
                        schema: None,
                        example: None,
                        required: false,
                    });
                    usage.push_field(&key);
                } else if body_aliases.iter().any(|a| a == receiver) {
                    usage.push_field(&key);
                }
            }

            "subscript" => {
                let (Some(value), Some(index)) = (
                    node.child_by_field_name("value"),
                    node.child_by_field_name("subscript"),
                ) else {
                    return;
                };
                let Some(key) = Constants::default().string_value(file, index) else {
                    return;
                };
                let receiver = file.text(value);
                if dialect.query.contains(&receiver) {
                    usage.push_query(ParamSpec::new(&key).required(), path_names);
                } else if dialect.headers.contains(&receiver) {
                    usage.push_header(&key);
                } else if dialect.meta_headers.contains(&receiver) {
                    if let Some(name) = meta_header_name(&key) {
                        usage.push_header(&name);
                    }
                } else if let Some(content_type) = dialect.body_content_type(receiver) {
                    usage.body.get_or_insert_with(|| BodySchema {
                        content_type: content_type.to_string(),
                        schema: None,
                        example: None,
                        required: true,
                    });
                    usage.push_field(&key);
                } else if body_aliases.iter().any(|a| a == receiver) {
                    usage.push_field(&key);
                }
            }

            "attribute" => {
                // A bare `request.json` / `request.form` read, with no key.
                if let Some(content_type) = body_source(file, node, dialect) {
                    usage.body.get_or_insert_with(|| BodySchema {
                        content_type: content_type.to_string(),
                        schema: None,
                        example: None,
                        required: false,
                    });
                }
            }

            _ => {}
        });

        usage
    }

    fn push_query(&mut self, spec: ParamSpec, path_names: &[&str]) {
        if path_names.contains(&spec.name.as_str())
            || self.query.iter().any(|q| q.name == spec.name)
        {
            return;
        }
        self.query.push(spec);
    }

    fn push_header(&mut self, name: &str) {
        if !self
            .headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case(name))
        {
            self.headers.push(ParamSpec::new(name));
        }
    }

    fn push_field(&mut self, name: &str) {
        if !self.body_fields.iter().any(|f| f == name) {
            self.body_fields.push(name.to_string());
        }
    }

    /// Write what was read onto the route. An `Authorization` header read by hand is an
    /// auth requirement, not a header to fill in.
    pub fn apply(mut self, fact: &mut RouteFact) {
        if let Some(index) = self
            .headers
            .iter()
            .position(|h| h.name.eq_ignore_ascii_case("authorization"))
        {
            self.headers.remove(index);
            if fact.auth.is_none() {
                fact.auth = Some(AuthRequirement::Bearer { format: None });
            }
        }

        fact.query_params = self.query;
        fact.headers = self.headers;
        if let Some(mut body) = self.body {
            if !self.body_fields.is_empty() {
                let properties: serde_json::Map<String, serde_json::Value> = self
                    .body_fields
                    .iter()
                    .map(|f| (f.clone(), serde_json::json!({})))
                    .collect();
                body.schema =
                    Some(serde_json::json!({ "type": "object", "properties": properties }));
                if body.content_type == "application/json" {
                    let example: serde_json::Map<String, serde_json::Value> = self
                        .body_fields
                        .iter()
                        .map(|f| (f.clone(), serde_json::Value::String(String::new())))
                        .collect();
                    body.example = Some(serde_json::Value::Object(example));
                }
            }
            fact.body = Some(body);
        }
    }
}

/// The content type a `request.…` expression reads, if it reads a body at all.
fn body_source(
    file: &ParsedFile,
    node: Node<'_>,
    dialect: &RequestDialect,
) -> Option<&'static str> {
    let text = match node.kind() {
        "call" => {
            let function = node.child_by_field_name("function")?;
            file.text(function)
        }
        "attribute" => file.text(node),
        _ => return None,
    };
    dialect.body_content_type(text)
}

/// `HTTP_X_API_KEY` → `X-Api-Key`; WSGI's spelling of a request header.
fn meta_header_name(key: &str) -> Option<String> {
    let rest = key.strip_prefix("HTTP_")?;
    Some(
        rest.split('_')
            .map(|part| {
                let lower = part.to_ascii_lowercase();
                let mut chars = lower.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join("-"),
    )
}

/// A literal default, as JSON.
pub fn literal(file: &ParsedFile, node: Node<'_>) -> Option<serde_json::Value> {
    if let Some(text) = Constants::default().string_value(file, node) {
        return Some(serde_json::Value::String(text));
    }
    match file.text(node) {
        "True" => Some(serde_json::Value::Bool(true)),
        "False" => Some(serde_json::Value::Bool(false)),
        "None" => Some(serde_json::Value::Null),
        raw => raw
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::SourceIndex;
    use crate::project::Language;

    fn parse(source: &str) -> ParsedFile {
        SourceIndex::new()
            .unwrap()
            .parse("main.py", Language::Python, source.to_string())
            .unwrap()
    }

    fn first_call(file: &ParsedFile) -> Node<'_> {
        let mut found = None;
        walk(file.root(), &mut |node| {
            if node.kind() == "call" && found.is_none() {
                found = Some(node);
            }
        });
        found.expect("no call in source")
    }

    #[test]
    fn folds_a_plain_string_constant() {
        let file = parse("PREFIX = \"/api/v1\"\n");
        let constants = Constants::collect(&file);
        assert_eq!(constants.get("PREFIX"), Some("/api/v1"));
    }

    #[test]
    fn folds_concatenation_and_chained_constants() {
        let file = parse("A = \"/api\"\nB = A + \"/v1\"\n");
        let constants = Constants::collect(&file);
        assert_eq!(constants.get("B"), Some("/api/v1"));
    }

    #[test]
    fn folds_an_f_string_whose_parts_are_known() {
        let file = parse("V = \"v1\"\nP = f\"/api/{V}\"\n");
        let constants = Constants::collect(&file);
        assert_eq!(constants.get("P"), Some("/api/v1"));
    }

    #[test]
    fn does_not_fold_anything_requiring_evaluation() {
        let file = parse("P = settings.API_PREFIX\nQ = os.environ['X']\nR = build_prefix()\n");
        let constants = Constants::collect(&file);
        assert_eq!(constants.get("P"), None);
        assert_eq!(constants.get("Q"), None);
        assert_eq!(constants.get("R"), None);
    }

    #[test]
    fn an_unfoldable_path_becomes_a_visible_gap() {
        let file = parse("app.include_router(r, prefix=settings.API_PREFIX)\n");
        let constants = Constants::collect(&file);
        let call = first_call(&file);
        let args = Arguments::of(&file, call);

        let template = constants.path_value(&file, args.keyword("prefix").unwrap());
        assert!(!template.is_resolved());
        assert_eq!(template.unresolved_exprs(), vec!["settings.API_PREFIX"]);
    }

    #[test]
    fn separates_positional_and_keyword_arguments() {
        let file = parse("router.get(\"/users\", response_model=UserOut, tags=[\"users\"])\n");
        let call = first_call(&file);
        let args = Arguments::of(&file, call);

        assert_eq!(args.positional.len(), 1);
        assert!(args.keyword("response_model").is_some());
        assert!(args.keyword("tags").is_some());
        assert!(args.keyword("missing").is_none());
    }

    #[test]
    fn reads_the_callee_in_both_forms() {
        let plain = parse("FastAPI()\n");
        assert_eq!(
            callee(&plain, first_call(&plain)),
            Some((None, "FastAPI".to_string()))
        );

        let attribute = parse("router.get(\"/x\")\n");
        assert_eq!(
            callee(&attribute, first_call(&attribute)),
            Some((Some("router".to_string()), "get".to_string()))
        );
    }

    #[test]
    fn reads_strings_out_of_lists() {
        let file =
            parse("router.api_route(\"/x\", methods=[\"GET\", \"POST\"], tags=[\"items\"])\n");
        let constants = Constants::collect(&file);
        let args = Arguments::of(&file, first_call(&file));

        assert_eq!(
            list_strings(&file, args.keyword("methods").unwrap(), &constants),
            vec!["GET", "POST"]
        );
        assert_eq!(
            first_list_string(&file, args.keyword("tags").unwrap(), &constants),
            Some("items".to_string())
        );
    }

    #[test]
    fn collects_every_import_form() {
        let file = parse(
            "from fastapi import FastAPI\n\
             from .users import router\n\
             from ..core import settings\n\
             from api.items import router as items_router\n\
             import api.orders\n\
             import api.carts as carts\n",
        );
        let imports = collect_imports(&file);

        let find = |local: &str| imports.iter().find(|i| i.local_name == local).cloned();

        let users = find("router").unwrap();
        assert_eq!(users.source, "users");
        assert_eq!(users.level, 1);
        assert_eq!(users.original.as_deref(), Some("router"));

        let core = find("settings").unwrap();
        assert_eq!(core.level, 2);

        let aliased = find("items_router").unwrap();
        assert_eq!(aliased.original.as_deref(), Some("router"));
        assert_eq!(aliased.source, "api.items");

        // `import api.orders` binds the dotted path and refers to a module, not a symbol.
        let orders = find("api.orders").unwrap();
        assert_eq!(orders.original, None);

        let carts = find("carts").unwrap();
        assert_eq!(carts.source, "api.carts");
        assert_eq!(carts.original, None);
    }
}
