//! OpenAPI import.
//!
//! Handles OpenAPI 3.1, 3.0, and Swagger 2.0. The two dialects differ in a handful of places
//! — where servers live, how a request body is declared, where security schemes are kept —
//! and those differences are absorbed here rather than leaking into the model.
//!
//! [`parse_openapi_value`] is the entry point runtime enrich will use in P5: FastAPI's
//! `app.openapi()` returns exactly this document, so that path needs no second extractor.

use crate::error::{ImportError, Result};
use crate::Imported;
use rl_model::{
    ApiKeyLocation, AuthRequirement, BodySchema, EndpointSpec, HttpMethod, Origin, ParamSpec,
    ParamStyle, PathTemplate, TypeHint,
};
use serde_json::{Map, Value};

/// How deep `$ref` resolution will go before giving up.
///
/// Recursive schemas (a `Node` with `children: [Node]`) are legitimate and common, so the
/// resolver has to stop somewhere rather than loop.
const MAX_REF_DEPTH: usize = 12;

const METHODS: [(&str, HttpMethod); 8] = [
    ("get", HttpMethod::Get),
    ("put", HttpMethod::Put),
    ("post", HttpMethod::Post),
    ("delete", HttpMethod::Delete),
    ("options", HttpMethod::Options),
    ("head", HttpMethod::Head),
    ("patch", HttpMethod::Patch),
    ("trace", HttpMethod::Trace),
];

/// A parsed specification.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenApiImport {
    pub title: String,
    pub version: String,
    /// Base URL candidates, in the order the document listed them.
    pub servers: Vec<String>,
    pub endpoints: Vec<EndpointSpec>,
}

/// Parse a specification from JSON or YAML text.
pub fn parse_openapi(text: &str) -> Result<Imported<OpenApiImport>> {
    if text.trim().is_empty() {
        return Err(ImportError::Empty);
    }

    let doc: Value = serde_json::from_str(text)
        .or_else(|_| yaml_serde::from_str(text))
        .map_err(|_| ImportError::NotStructured)?;

    parse_openapi_value(&doc)
}

/// Parse an already-decoded specification.
pub fn parse_openapi_value(doc: &Value) -> Result<Imported<OpenApiImport>> {
    let mut warnings = Vec::new();

    let swagger_2 = doc
        .get("swagger")
        .and_then(Value::as_str)
        .is_some_and(|v| v.starts_with('2'));
    let openapi_3 = doc.get("openapi").and_then(Value::as_str);

    match (swagger_2, openapi_3) {
        (true, _) => {}
        (false, Some(v)) if v.starts_with('3') => {}
        (false, Some(v)) => {
            return Err(ImportError::UnsupportedVersion {
                version: v.to_string(),
            })
        }
        (false, None) => return Err(ImportError::NotOpenApi),
    }

    let info = doc.get("info");
    let title = info
        .and_then(|i| i.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("API")
        .to_string();
    let version = info
        .and_then(|i| i.get("version"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let servers = extract_servers(doc, swagger_2);
    let schemes = security_schemes(doc, swagger_2);
    let global_security = doc.get("security");

    let mut endpoints = Vec::new();

    let Some(paths) = doc.get("paths").and_then(Value::as_object) else {
        warnings.push("the document declares no paths".into());
        return Ok(Imported {
            value: OpenApiImport {
                title,
                version,
                servers,
                endpoints,
            },
            warnings,
        });
    };

    for (raw_path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };

        // Parameters declared on the path apply to every operation under it.
        let shared_params = item.get("parameters").and_then(Value::as_array);

        for (key, method) in &METHODS {
            let Some(operation) = item.get(*key).and_then(Value::as_object) else {
                continue;
            };

            let template = PathTemplate::parse(raw_path, ParamStyle::Braces);
            let mut spec = EndpointSpec::new(
                method.clone(),
                template,
                Origin::OpenApi {
                    document: Some(title.clone()),
                },
            );

            spec.summary = operation
                .get("summary")
                .or_else(|| operation.get("operationId"))
                .and_then(Value::as_str)
                .map(str::to_string);
            spec.description = operation
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string);
            spec.group = operation
                .get("tags")
                .and_then(Value::as_array)
                .and_then(|t| t.first())
                .and_then(Value::as_str)
                .map(str::to_string);

            // Path-level parameters first, so an operation-level one of the same name and
            // location overrides it — which is what the specification requires.
            let mut path_params: Vec<ParamSpec> = Vec::new();
            let mut query_params: Vec<ParamSpec> = Vec::new();
            let mut header_params: Vec<ParamSpec> = Vec::new();
            let mut body_from_v2: Option<BodySchema> = None;

            for source in [
                shared_params,
                operation.get("parameters").and_then(Value::as_array),
            ]
            .into_iter()
            .flatten()
            {
                for parameter in source {
                    let parameter = resolve_refs(parameter, doc, 0, &mut Vec::new());
                    let Some(name) = parameter.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let location = parameter
                        .get("in")
                        .and_then(Value::as_str)
                        .unwrap_or("query");

                    // Swagger 2.0 declares request bodies as a parameter.
                    if location == "body" {
                        let schema = parameter.get("schema").cloned().unwrap_or(Value::Null);
                        body_from_v2 = Some(BodySchema {
                            content_type: "application/json".to_string(),
                            example: Some(example_from_schema(&schema, 0)),
                            schema: Some(schema),
                            required: parameter
                                .get("required")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        });
                        continue;
                    }

                    let spec_param = build_param(&parameter, name);
                    let bucket = match location {
                        "path" => &mut path_params,
                        "header" => &mut header_params,
                        "formData" => &mut query_params,
                        _ => &mut query_params,
                    };
                    match bucket.iter_mut().find(|p| p.name == spec_param.name) {
                        Some(existing) => *existing = spec_param,
                        None => bucket.push(spec_param),
                    }
                }
            }

            // Path parameters are already seeded from the template; enrich rather than
            // replace, so a parameter the document forgot to declare is not lost.
            for declared in path_params {
                // A `format: path` parameter spans `/`s — Flask's `<path:name>` reported
                // by runtime enrich — and the template segment should say so.
                if declared.ty == Some(TypeHint::Path) {
                    for segment in &mut spec.path.segments {
                        if let rl_model::PathSegment::Param {
                            name,
                            ty,
                            catch_all,
                            ..
                        } = segment
                        {
                            if *name == declared.name {
                                *ty = Some(TypeHint::Path);
                                *catch_all = true;
                            }
                        }
                    }
                }
                match spec
                    .path_params
                    .iter_mut()
                    .find(|p| p.name == declared.name)
                {
                    Some(existing) => *existing = declared,
                    None => spec.path_params.push(declared),
                }
            }
            spec.query_params = query_params;
            spec.headers = header_params;

            spec.body = body_from_v2.or_else(|| extract_request_body(operation, doc));

            spec.auth = operation
                .get("security")
                .or(global_security)
                .and_then(|s| auth_from_security(s, &schemes));

            if operation.get("deprecated").and_then(Value::as_bool) == Some(true) {
                spec.metadata.insert("deprecated".into(), Value::Bool(true));
            }

            endpoints.push(spec);
        }
    }

    if endpoints.is_empty() {
        warnings.push("no operations were found in the document".into());
    }
    if servers.is_empty() {
        warnings.push("the document names no server, so a base URL must be supplied".into());
    }

    Ok(Imported {
        value: OpenApiImport {
            title,
            version,
            servers,
            endpoints,
        },
        warnings,
    })
}

fn extract_servers(doc: &Value, swagger_2: bool) -> Vec<String> {
    if swagger_2 {
        let host = doc.get("host").and_then(Value::as_str).unwrap_or_default();
        let base = doc.get("basePath").and_then(Value::as_str).unwrap_or("");
        if host.is_empty() {
            return Vec::new();
        }
        let schemes = doc
            .get("schemes")
            .and_then(Value::as_array)
            .map(|s| {
                s.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec!["https".to_string()]);

        return schemes
            .into_iter()
            .map(|scheme| format!("{scheme}://{host}{base}"))
            .collect();
    }

    doc.get("servers")
        .and_then(Value::as_array)
        .map(|servers| {
            servers
                .iter()
                .filter_map(|s| s.get("url").and_then(Value::as_str))
                .map(|url| url.trim_end_matches('/').to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn build_param(parameter: &Value, name: &str) -> ParamSpec {
    let mut spec = ParamSpec::new(name);

    spec.required = parameter
        .get("required")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    spec.description = parameter
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);

    // 3.x nests the type in `schema`; 2.0 puts it on the parameter itself.
    let schema = parameter.get("schema").unwrap_or(parameter);

    if let Some(ty) = schema.get("type").and_then(Value::as_str) {
        let format = schema.get("format").and_then(Value::as_str);
        spec.ty = Some(type_hint(ty, format));
    }
    if let Some(default) = schema.get("default") {
        spec.default = Some(default.clone());
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        spec.enum_values = values.clone();
    }

    spec
}

fn type_hint(ty: &str, format: Option<&str>) -> TypeHint {
    match (ty, format) {
        (_, Some("uuid")) => TypeHint::Uuid,
        (_, Some("path")) => TypeHint::Path,
        (_, Some("date")) => TypeHint::Date,
        (_, Some("date-time")) => TypeHint::DateTime,
        ("integer", _) => TypeHint::Integer,
        ("number", _) => TypeHint::Number,
        ("boolean", _) => TypeHint::Boolean,
        ("string", _) => TypeHint::String,
        (other, _) => TypeHint::Other(other.to_string()),
    }
}

fn extract_request_body(operation: &Map<String, Value>, doc: &Value) -> Option<BodySchema> {
    let body = operation.get("requestBody")?;
    let body = resolve_refs(body, doc, 0, &mut Vec::new());
    let content = body.get("content")?.as_object()?;

    // Prefer JSON when the operation offers a choice, since that is what the body editor is
    // set up for; otherwise take whatever is first.
    let (content_type, media) = content
        .iter()
        .find(|(ct, _)| ct.contains("json"))
        .or_else(|| content.iter().next())?;

    let schema = media
        .get("schema")
        .map(|s| resolve_refs(s, doc, 0, &mut Vec::new()));

    let example = media
        .get("example")
        .cloned()
        .or_else(|| {
            media
                .get("examples")
                .and_then(Value::as_object)
                .and_then(|examples| examples.values().next())
                .and_then(|first| first.get("value"))
                .cloned()
        })
        .or_else(|| schema.as_ref().map(|s| example_from_schema(s, 0)));

    Some(BodySchema {
        content_type: content_type.clone(),
        schema,
        example,
        required: body
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn security_schemes(doc: &Value, swagger_2: bool) -> Map<String, Value> {
    let node = if swagger_2 {
        doc.get("securityDefinitions")
    } else {
        doc.get("components").and_then(|c| c.get("securitySchemes"))
    };

    node.and_then(Value::as_object).cloned().unwrap_or_default()
}

fn auth_from_security(security: &Value, schemes: &Map<String, Value>) -> Option<AuthRequirement> {
    let requirements = security.as_array()?;
    // An empty `security: []` on an operation means the endpoint is deliberately public.
    let first = requirements.first()?.as_object()?;
    let (name, scopes) = first.iter().next()?;

    let Some(scheme) = schemes.get(name) else {
        // The document referenced a scheme it never defined. Saying "auth of some kind" is
        // more useful than saying nothing.
        return Some(AuthRequirement::Unknown { hint: name.clone() });
    };

    let kind = scheme.get("type").and_then(Value::as_str).unwrap_or("");

    Some(match kind {
        "http" => match scheme.get("scheme").and_then(Value::as_str) {
            Some(s) if s.eq_ignore_ascii_case("bearer") => AuthRequirement::Bearer {
                format: scheme
                    .get("bearerFormat")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
            Some(s) if s.eq_ignore_ascii_case("basic") => AuthRequirement::Basic,
            other => AuthRequirement::Unknown {
                hint: other.unwrap_or("http").to_string(),
            },
        },
        // Swagger 2.0 spells basic auth as its own type.
        "basic" => AuthRequirement::Basic,
        "apiKey" => AuthRequirement::ApiKey {
            name: scheme
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("api_key")
                .to_string(),
            location: match scheme.get("in").and_then(Value::as_str) {
                Some("query") => ApiKeyLocation::Query,
                Some("cookie") => ApiKeyLocation::Cookie,
                _ => ApiKeyLocation::Header,
            },
        },
        "oauth2" => AuthRequirement::OAuth2 {
            scopes: scopes
                .as_array()
                .map(|s| {
                    s.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        },
        "openIdConnect" => AuthRequirement::OAuth2 { scopes: Vec::new() },
        other => AuthRequirement::Unknown {
            hint: other.to_string(),
        },
    })
}

/// Replace `{"$ref": "#/..."}` with what it points at.
///
/// Only local references are followed. Fetching a remote `$ref` would mean a document could
/// make RouteLens issue network requests the user never asked for, which is not a trade this
/// importer makes — see `docs/security.md`.
fn resolve_refs(value: &Value, root: &Value, depth: usize, seen: &mut Vec<String>) -> Value {
    if depth > MAX_REF_DEPTH {
        return Value::Object(Map::new());
    }

    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("$ref") {
                if !reference.starts_with("#/") {
                    // Remote reference: left in place so it is visible rather than silently
                    // replaced by an empty schema.
                    return value.clone();
                }
                if seen.contains(reference) {
                    return Value::Object(Map::new());
                }
                let Some(target) = resolve_pointer(root, reference) else {
                    return Value::Object(Map::new());
                };
                seen.push(reference.clone());
                let resolved = resolve_refs(target, root, depth + 1, seen);
                seen.pop();
                return resolved;
            }

            Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), resolve_refs(v, root, depth + 1, seen)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| resolve_refs(v, root, depth + 1, seen))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Resolve a JSON pointer such as `#/components/schemas/User`.
fn resolve_pointer<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in reference.trim_start_matches("#/").split('/') {
        if segment.is_empty() {
            continue;
        }
        // JSON pointer escaping: ~1 is `/`, ~0 is `~`.
        let segment = segment.replace("~1", "/").replace("~0", "~");
        current = match current {
            Value::Object(map) => map.get(&segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Build a worked example from a schema, to pre-fill the body editor.
///
/// An empty editor with a schema hidden behind a tab is worse than a filled-in skeleton the
/// developer can edit down.
fn example_from_schema(schema: &Value, depth: usize) -> Value {
    if depth > MAX_REF_DEPTH {
        return Value::Null;
    }

    if let Some(example) = schema.get("example").or_else(|| schema.get("default")) {
        return example.clone();
    }
    if let Some(first) = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|e| e.first())
    {
        return first.clone();
    }
    // A composed schema: the first branch is a reasonable representative.
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(first) = schema
            .get(key)
            .and_then(Value::as_array)
            .and_then(|a| a.first())
        {
            return example_from_schema(first, depth + 1);
        }
    }

    let ty = schema.get("type").and_then(Value::as_str);
    let format = schema.get("format").and_then(Value::as_str);

    match (ty, format) {
        (Some("object"), _) | (None, _) if schema.get("properties").is_some() => {
            let properties = schema
                .get("properties")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            Value::Object(
                properties
                    .iter()
                    .map(|(name, sub)| (name.clone(), example_from_schema(sub, depth + 1)))
                    .collect(),
            )
        }
        (Some("array"), _) => {
            let items = schema.get("items").cloned().unwrap_or(Value::Null);
            Value::Array(vec![example_from_schema(&items, depth + 1)])
        }
        (Some("integer"), _) => Value::from(0),
        (Some("number"), _) => Value::from(0.0),
        (Some("boolean"), _) => Value::Bool(false),
        (Some("string"), Some("date-time")) => Value::from("1970-01-01T00:00:00Z"),
        (Some("string"), Some("date")) => Value::from("1970-01-01"),
        (Some("string"), Some("uuid")) => Value::from("00000000-0000-0000-0000-000000000000"),
        (Some("string"), Some("email")) => Value::from("user@example.com"),
        (Some("string"), _) => Value::from(""),
        (Some("null"), _) => Value::Null,
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn petstore() -> Value {
        json!({
            "openapi": "3.0.3",
            "info": {"title": "Petstore", "version": "1.0.0"},
            "servers": [{"url": "https://api.example.com/v1"}],
            "components": {
                "securitySchemes": {
                    "bearerAuth": {"type": "http", "scheme": "bearer", "bearerFormat": "JWT"}
                },
                "schemas": {
                    "Pet": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "integer"},
                            "name": {"type": "string"},
                            "tags": {"type": "array", "items": {"type": "string"}}
                        }
                    }
                }
            },
            "security": [{"bearerAuth": []}],
            "paths": {
                "/pets": {
                    "get": {
                        "summary": "List pets",
                        "tags": ["pets"],
                        "parameters": [
                            {"name": "limit", "in": "query", "required": false,
                             "schema": {"type": "integer", "default": 20}}
                        ]
                    },
                    "post": {
                        "summary": "Create a pet",
                        "tags": ["pets"],
                        "requestBody": {
                            "required": true,
                            "content": {
                                "application/json": {
                                    "schema": {"$ref": "#/components/schemas/Pet"}
                                }
                            }
                        }
                    }
                },
                "/pets/{petId}": {
                    "parameters": [
                        {"name": "petId", "in": "path", "required": true,
                         "schema": {"type": "string", "format": "uuid"}}
                    ],
                    "get": {"summary": "Get a pet", "tags": ["pets"]}
                }
            }
        })
    }

    fn parse(doc: &Value) -> OpenApiImport {
        parse_openapi_value(doc).unwrap().value
    }

    #[test]
    fn operations_become_endpoints() {
        let api = parse(&petstore());
        assert_eq!(api.title, "Petstore");
        assert_eq!(api.endpoints.len(), 3);

        let list = api
            .endpoints
            .iter()
            .find(|e| e.method == HttpMethod::Get && e.path.render(ParamStyle::Braces) == "/pets")
            .unwrap();
        assert_eq!(list.summary.as_deref(), Some("List pets"));
        assert_eq!(list.group.as_deref(), Some("pets"));
    }

    #[test]
    fn servers_become_base_url_candidates() {
        assert_eq!(
            parse(&petstore()).servers,
            vec!["https://api.example.com/v1"]
        );
    }

    #[test]
    fn query_parameters_carry_type_and_default() {
        let api = parse(&petstore());
        let list = api
            .endpoints
            .iter()
            .find(|e| e.query_params.iter().any(|p| p.name == "limit"))
            .unwrap();
        let limit = &list.query_params[0];
        assert_eq!(limit.ty.as_ref(), Some(&TypeHint::Integer));
        assert_eq!(limit.default, Some(json!(20)));
        assert!(!limit.required);
    }

    #[test]
    fn path_level_parameters_apply_to_every_operation_beneath() {
        let api = parse(&petstore());
        let get_one = api
            .endpoints
            .iter()
            .find(|e| e.path.render(ParamStyle::Braces) == "/pets/{petId}")
            .unwrap();

        let param = get_one
            .path_params
            .iter()
            .find(|p| p.name == "petId")
            .unwrap();
        assert!(param.required);
        assert_eq!(param.ty.as_ref(), Some(&TypeHint::Uuid));
    }

    #[test]
    fn a_ref_in_a_request_body_is_resolved_and_gives_an_example() {
        let api = parse(&petstore());
        let create = api
            .endpoints
            .iter()
            .find(|e| e.method == HttpMethod::Post)
            .unwrap();

        let body = create.body.as_ref().unwrap();
        assert_eq!(body.content_type, "application/json");
        assert!(body.required);

        // The $ref was followed, not left dangling.
        let schema = body.schema.as_ref().unwrap();
        assert!(schema.get("properties").is_some());

        let example = body.example.as_ref().unwrap();
        assert_eq!(example.get("id"), Some(&json!(0)));
        assert_eq!(example.get("name"), Some(&json!("")));
        assert_eq!(example.get("tags"), Some(&json!([""])));
    }

    #[test]
    fn global_security_applies_to_operations() {
        let api = parse(&petstore());
        let list = &api.endpoints[0];
        assert_eq!(
            list.auth,
            Some(AuthRequirement::Bearer {
                format: Some("JWT".into())
            })
        );
    }

    #[test]
    fn an_operation_can_override_global_security() {
        let mut doc = petstore();
        doc["paths"]["/pets"]["get"]["security"] = json!([{"apiKeyAuth": []}]);
        doc["components"]["securitySchemes"]["apiKeyAuth"] =
            json!({"type": "apiKey", "name": "X-API-Key", "in": "header"});

        let api = parse(&doc);
        let list = api
            .endpoints
            .iter()
            .find(|e| e.method == HttpMethod::Get && e.path.render(ParamStyle::Braces) == "/pets")
            .unwrap();

        assert_eq!(
            list.auth,
            Some(AuthRequirement::ApiKey {
                name: "X-API-Key".into(),
                location: ApiKeyLocation::Header
            })
        );
    }

    #[test]
    fn a_security_scheme_that_was_never_defined_is_still_reported() {
        let doc = json!({
            "openapi": "3.0.0",
            "info": {"title": "X", "version": "1"},
            "security": [{"ghost": []}],
            "paths": {"/x": {"get": {}}}
        });
        let api = parse(&doc);
        assert_eq!(
            api.endpoints[0].auth,
            Some(AuthRequirement::Unknown {
                hint: "ghost".into()
            })
        );
    }

    #[test]
    fn a_recursive_schema_terminates() {
        let doc = json!({
            "openapi": "3.0.0",
            "info": {"title": "X", "version": "1"},
            "components": {"schemas": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "children": {"type": "array", "items": {"$ref": "#/components/schemas/Node"}}
                    }
                }
            }},
            "paths": {"/nodes": {"post": {"requestBody": {"content": {"application/json": {
                "schema": {"$ref": "#/components/schemas/Node"}
            }}}}}}
        });

        // The point is that this returns at all.
        let api = parse(&doc);
        assert!(api.endpoints[0].body.is_some());
    }

    #[test]
    fn swagger_2_documents_are_understood() {
        let doc = json!({
            "swagger": "2.0",
            "info": {"title": "Legacy", "version": "1.0"},
            "host": "api.legacy.test",
            "basePath": "/v2",
            "schemes": ["https"],
            "securityDefinitions": {"basicAuth": {"type": "basic"}},
            "definitions": {"User": {"type": "object", "properties": {"name": {"type": "string"}}}},
            "paths": {
                "/users": {
                    "post": {
                        "summary": "Create user",
                        "security": [{"basicAuth": []}],
                        "parameters": [
                            {"name": "verbose", "in": "query", "type": "boolean"},
                            {"name": "body", "in": "body", "required": true,
                             "schema": {"$ref": "#/definitions/User"}}
                        ]
                    }
                }
            }
        });

        let api = parse(&doc);
        assert_eq!(api.servers, vec!["https://api.legacy.test/v2"]);

        let create = &api.endpoints[0];
        assert_eq!(create.method, HttpMethod::Post);
        assert_eq!(create.auth, Some(AuthRequirement::Basic));
        assert_eq!(create.query_params[0].name, "verbose");

        // The 2.0 `in: body` parameter became a request body, with its $ref resolved.
        let body = create.body.as_ref().unwrap();
        assert!(body.required);
        assert_eq!(body.example.as_ref().unwrap().get("name"), Some(&json!("")));
    }

    #[test]
    fn yaml_documents_parse() {
        let text = "
openapi: 3.0.0
info:
  title: YAML API
  version: '1.0'
paths:
  /health:
    get:
      summary: Health check
";
        let api = parse_openapi(text).unwrap().value;
        assert_eq!(api.title, "YAML API");
        assert_eq!(api.endpoints.len(), 1);
    }

    #[test]
    fn endpoints_carry_the_openapi_origin_and_no_source_location() {
        let api = parse(&petstore());
        let endpoint = &api.endpoints[0];
        assert!(matches!(endpoint.origin, Origin::OpenApi { .. }));
        assert!(endpoint.origin.is_authoritative());
        assert!(
            endpoint.source.is_none(),
            "a document knows no source location"
        );
    }

    #[test]
    fn a_document_with_no_servers_warns_rather_than_inventing_one() {
        let doc = json!({
            "openapi": "3.0.0",
            "info": {"title": "X", "version": "1"},
            "paths": {"/x": {"get": {}}}
        });
        let imported = parse_openapi_value(&doc).unwrap();
        assert!(imported.warnings.iter().any(|w| w.contains("server")));
    }

    #[test]
    fn a_deprecated_operation_is_flagged() {
        let mut doc = petstore();
        doc["paths"]["/pets"]["get"]["deprecated"] = json!(true);
        let api = parse(&doc);
        let list = api
            .endpoints
            .iter()
            .find(|e| e.method == HttpMethod::Get && e.path.render(ParamStyle::Braces) == "/pets")
            .unwrap();
        assert_eq!(list.metadata.get("deprecated"), Some(&json!(true)));
    }

    #[test]
    fn non_openapi_input_is_refused_clearly() {
        assert!(matches!(
            parse_openapi(r#"{"hello": "world"}"#),
            Err(ImportError::NotOpenApi)
        ));
        assert!(matches!(parse_openapi("   "), Err(ImportError::Empty)));
        assert!(matches!(
            parse_openapi_value(&json!({"openapi": "4.0.0", "paths": {}})),
            Err(ImportError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn a_json_pointer_with_escapes_resolves() {
        let root = json!({"a~b": {"c/d": 42}});
        assert_eq!(resolve_pointer(&root, "#/a~0b/c~1d"), Some(&json!(42)));
    }

    #[test]
    fn a_remote_ref_is_left_visible_rather_than_emptied() {
        let doc = json!({"$ref": "https://example.com/schema.json"});
        let resolved = resolve_refs(&doc, &json!({}), 0, &mut Vec::new());
        assert_eq!(resolved, doc);
    }
}
