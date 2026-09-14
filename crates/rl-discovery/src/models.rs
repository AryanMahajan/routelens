//! Data models read from source, so a request body can be filled in before anything runs.
//!
//! A FastAPI handler says `payload: UserCreate`; the shape of `UserCreate` is a class in
//! another file. Adapters report every such class they see as a [`ModelFact`] — its fields,
//! annotations and defaults — and after routes are resolved, [`ModelIndex::fill`] turns a
//! body that only knows its model's name into a JSON Schema and a worked example. Runtime
//! enrich does this exactly; this is the static best effort that needs nothing to run.

use rl_model::BodySchema;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A class with annotated fields — a Pydantic model, typically.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelFact {
    pub name: String,
    pub module: PathBuf,
    /// Base class names as written, so inherited fields can be found.
    pub bases: Vec<String>,
    pub fields: Vec<ModelField>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelField {
    pub name: String,
    /// The annotation as written: `str`, `list[Item]`, `Optional[int]`.
    pub annotation: String,
    /// A literal default, when the source states one.
    pub default: Option<Value>,
    pub required: bool,
}

/// Nested models deeper than this are cut off — a self-referential `Node.children`.
const MAX_DEPTH: usize = 4;

/// Every model a scan saw, by name.
#[derive(Debug, Default)]
pub struct ModelIndex {
    by_name: BTreeMap<String, Vec<ModelFact>>,
}

impl ModelIndex {
    pub fn new(models: Vec<ModelFact>) -> Self {
        let mut by_name: BTreeMap<String, Vec<ModelFact>> = BTreeMap::new();
        for model in models {
            by_name.entry(model.name.clone()).or_default().push(model);
        }
        ModelIndex { by_name }
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// The model called `name`. Two files declaring the same class name is unusual and
    /// unresolvable without the import graph; the first wins rather than none.
    fn get(&self, name: &str) -> Option<&ModelFact> {
        self.by_name.get(name).and_then(|m| m.first())
    }

    /// Give a body that names its model — `{"type": "object", "title": "UserCreate"}` — the
    /// model's fields as a schema, and an example to pre-fill the editor. A body that
    /// already has properties (from an OpenAPI document, say) is left alone.
    pub fn fill(&self, body: &mut BodySchema) {
        let Some(title) = body
            .schema
            .as_ref()
            .filter(|s| s.get("properties").is_none())
            .and_then(|s| s.get("title"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let Some((schema, example)) = self.describe(title, 0) else {
            return;
        };
        body.schema = Some(schema);
        if body.example.is_none() {
            body.example = Some(example);
        }
    }

    /// `(schema, example)` for a model, fields of its bases first.
    fn describe(&self, name: &str, depth: usize) -> Option<(Value, Value)> {
        let model = self.get(name)?;
        if depth > MAX_DEPTH {
            return None;
        }
        let mut properties = Map::new();
        let mut required = Vec::new();
        let mut example = Map::new();
        for base in &model.bases {
            if let Some((schema, ex)) = self.describe(base, depth + 1) {
                if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                    properties.extend(props.clone());
                }
                if let Some(req) = schema.get("required").and_then(Value::as_array) {
                    required.extend(req.iter().filter_map(Value::as_str).map(String::from));
                }
                if let Some(ex) = ex.as_object() {
                    example.extend(ex.clone());
                }
            }
        }
        for field in &model.fields {
            let (mut schema, mut value) = self.annotation(&field.annotation, depth + 1);
            if let Some(default) = &field.default {
                if !default.is_null() {
                    value = default.clone();
                }
                schema
                    .as_object_mut()
                    .map(|s| s.insert("default".into(), default.clone()));
            }
            properties.insert(field.name.clone(), schema);
            example.insert(field.name.clone(), value);
            if field.required {
                required.push(field.name.clone());
            }
        }
        let mut schema = json!({ "type": "object", "title": name, "properties": properties });
        if !required.is_empty() {
            schema["required"] = Value::Array(required.into_iter().map(Value::from).collect());
        }
        Some((schema, Value::Object(example)))
    }

    /// `(schema, example)` for a type annotation as Python writes it.
    fn annotation(&self, annotation: &str, depth: usize) -> (Value, Value) {
        let text = annotation.trim();
        // `X | None` and `Optional[X]` describe X; `Annotated[X, ...]` too.
        if let Some(inner) = text
            .strip_suffix("| None")
            .or_else(|| text.strip_prefix("None |"))
        {
            return self.annotation(inner, depth);
        }
        if let Some((outer, inner)) = generic(text) {
            match outer {
                "Optional" | "Annotated" | "Required" | "NotRequired" | "Final" => {
                    let first = split_top_level(inner)
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    return self.annotation(&first, depth);
                }
                "Union" => {
                    let first = split_top_level(inner)
                        .into_iter()
                        .find(|p| p.trim() != "None")
                        .unwrap_or_default();
                    return self.annotation(&first, depth);
                }
                "list" | "List" | "Sequence" | "set" | "Set" | "frozenset" | "tuple" | "Tuple"
                | "Iterable" => {
                    let first = split_top_level(inner)
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    let (items, item) = self.annotation(&first, depth);
                    return (json!({ "type": "array", "items": items }), json!([item]));
                }
                "dict" | "Dict" | "Mapping" => {
                    let parts = split_top_level(inner);
                    let (values, _) =
                        self.annotation(parts.get(1).map(String::as_str).unwrap_or(""), depth);
                    return (
                        json!({ "type": "object", "additionalProperties": values }),
                        json!({}),
                    );
                }
                "Literal" => {
                    let values: Vec<Value> = split_top_level(inner)
                        .iter()
                        .filter_map(|v| literal_text(v))
                        .collect();
                    let first = values
                        .first()
                        .cloned()
                        .unwrap_or(Value::String(String::new()));
                    return (json!({ "enum": values }), first);
                }
                _ => {}
            }
        }
        let base = text.rsplit('.').next().unwrap_or(text);
        match base {
            "str" | "SecretStr" | "constr" | "StrictStr" => {
                (json!({ "type": "string" }), json!("string"))
            }
            "int" | "conint" | "StrictInt" | "PositiveInt" | "NonNegativeInt" => {
                (json!({ "type": "integer" }), json!(0))
            }
            "float" | "Decimal" | "confloat" | "StrictFloat" | "PositiveFloat" => {
                (json!({ "type": "number" }), json!(0.0))
            }
            "bool" | "StrictBool" => (json!({ "type": "boolean" }), json!(false)),
            "bytes" => (json!({ "type": "string", "format": "binary" }), json!("")),
            "EmailStr" | "NameEmail" => (
                json!({ "type": "string", "format": "email" }),
                json!("user@example.com"),
            ),
            "HttpUrl" | "AnyUrl" | "AnyHttpUrl" | "Url" => (
                json!({ "type": "string", "format": "uri" }),
                json!("https://example.com"),
            ),
            "UUID" | "UUID4" => (
                json!({ "type": "string", "format": "uuid" }),
                json!("3fa85f64-5717-4562-b3fc-2c963f66afa6"),
            ),
            "datetime" => (
                json!({ "type": "string", "format": "date-time" }),
                json!("2024-01-01T00:00:00Z"),
            ),
            "date" => (
                json!({ "type": "string", "format": "date" }),
                json!("2024-01-01"),
            ),
            "time" => (
                json!({ "type": "string", "format": "time" }),
                json!("00:00:00"),
            ),
            "Any" | "object" | "" => (json!({}), json!(null)),
            "dict" | "Dict" | "Json" => (json!({ "type": "object" }), json!({})),
            "list" | "List" => (json!({ "type": "array" }), json!([])),
            other => match self.describe(other, depth) {
                Some(pair) => pair,
                // An unknown name: an enum, a type alias, a model in a file the scan did not
                // parse. A string is the most likely shape and the easiest to correct.
                None => (json!({ "type": "string", "title": other }), json!("string")),
            },
        }
    }
}

/// `list[Item]` → `("list", "Item")`.
fn generic(text: &str) -> Option<(&str, &str)> {
    let open = text.find('[')?;
    let inner = text[open + 1..].strip_suffix(']')?;
    Some((text[..open].trim().rsplit('.').next().unwrap_or(""), inner))
}

/// Split `str, list[int], dict[str, int]` on the commas that are not inside brackets.
fn split_top_level(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for ch in text.chars() {
        match ch {
            '[' | '(' => {
                depth += 1;
                current.push(ch);
            }
            ']' | ')' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

/// A Python literal inside `Literal[...]`, as JSON.
fn literal_text(text: &str) -> Option<Value> {
    let t = text.trim();
    if (t.starts_with('"') && t.ends_with('"') || t.starts_with('\'') && t.ends_with('\''))
        && t.len() >= 2
    {
        return Some(Value::String(t[1..t.len() - 1].to_string()));
    }
    match t {
        "True" => Some(json!(true)),
        "False" => Some(json!(false)),
        "None" => Some(Value::Null),
        _ => t
            .parse::<i64>()
            .ok()
            .map(Value::from)
            .or_else(|| t.parse::<f64>().ok().map(|f| json!(f))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(name: &str, bases: &[&str], fields: &[(&str, &str, Option<Value>)]) -> ModelFact {
        ModelFact {
            name: name.into(),
            module: PathBuf::from("app/schemas.py"),
            bases: bases.iter().map(|b| b.to_string()).collect(),
            fields: fields
                .iter()
                .map(|(n, a, d)| ModelField {
                    name: n.to_string(),
                    annotation: a.to_string(),
                    default: d.clone(),
                    required: d.is_none(),
                })
                .collect(),
        }
    }

    fn body(title: &str) -> BodySchema {
        BodySchema {
            content_type: "application/json".into(),
            schema: Some(json!({ "type": "object", "title": title })),
            example: None,
            required: true,
        }
    }

    #[test]
    fn a_model_becomes_a_schema_and_an_example_with_nesting_and_inheritance() {
        let index = ModelIndex::new(vec![
            model("Base", &["BaseModel"], &[("id", "UUID", None)]),
            model(
                "Address",
                &["BaseModel"],
                &[
                    ("city", "str", None),
                    ("zip", "Optional[str]", Some(Value::Null)),
                ],
            ),
            model(
                "UserCreate",
                &["Base"],
                &[
                    ("name", "str", None),
                    ("email", "EmailStr", None),
                    ("age", "int", Some(json!(18))),
                    ("tags", "list[str]", None),
                    ("address", "Address | None", Some(Value::Null)),
                    ("role", "Literal[\"admin\", \"user\"]", None),
                ],
            ),
        ]);
        let mut b = body("UserCreate");
        index.fill(&mut b);

        let schema = b.schema.unwrap();
        assert_eq!(schema["title"], "UserCreate");
        assert_eq!(
            schema["properties"]["id"]["format"], "uuid",
            "inherited from Base"
        );
        assert_eq!(schema["properties"]["tags"]["type"], "array");
        assert_eq!(
            schema["properties"]["address"]["properties"]["city"]["type"],
            "string"
        );
        assert_eq!(schema["properties"]["age"]["default"], 18);
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "name") && !required.iter().any(|r| r == "age"));

        assert_eq!(
            b.example.unwrap(),
            json!({
                "id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
                "name": "string",
                "email": "user@example.com",
                "age": 18,
                "tags": ["string"],
                "address": { "city": "string", "zip": "string" },
                "role": "admin"
            })
        );
    }

    #[test]
    fn a_body_that_already_has_properties_or_names_no_known_model_is_left_alone() {
        let index = ModelIndex::new(vec![model("Thing", &["BaseModel"], &[("x", "int", None)])]);

        let mut unknown = body("Elsewhere");
        index.fill(&mut unknown);
        assert_eq!(unknown.schema.unwrap()["title"], "Elsewhere");
        assert!(unknown.example.is_none());

        let mut described = BodySchema {
            content_type: "application/json".into(),
            schema: Some(json!({ "title": "Thing", "properties": { "y": { "type": "string" } } })),
            example: Some(json!({ "y": "from openapi" })),
            required: true,
        };
        index.fill(&mut described);
        assert_eq!(described.example.unwrap()["y"], "from openapi");
    }

    #[test]
    fn a_self_referential_model_stops_at_a_depth() {
        let index = ModelIndex::new(vec![model(
            "Node",
            &["BaseModel"],
            &[("children", "list[Node]", None)],
        )]);
        let mut b = body("Node");
        index.fill(&mut b);
        // Terminates, and the innermost level falls back to a string placeholder.
        assert!(serde_json::to_string(&b.example.unwrap())
            .unwrap()
            .contains("children"));
    }

    #[test]
    fn annotations_are_split_on_top_level_commas_only() {
        assert_eq!(
            split_top_level("str, dict[str, int], list[tuple[int, int]]"),
            vec!["str", "dict[str, int]", "list[tuple[int, int]]"]
        );
        assert_eq!(generic("typing.List[int]"), Some(("List", "int")));
    }
}
