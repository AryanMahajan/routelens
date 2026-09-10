//! Path templates.
//!
//! A discovered path is a list of segments, not a string. That distinction is what lets
//! RouteLens compose router prefixes across files, and — more importantly — admit when it
//! could not work a segment out. See [`PathSegment::Unresolved`].

use serde::{Deserialize, Serialize};
use std::fmt;

/// How a framework spells a path parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamStyle {
    /// `{user_id}` — FastAPI, Starlette, OpenAPI.
    Braces,
    /// `:id` — Express.
    Colon,
    /// `<int:user_id>` — Flask, Werkzeug, Django.
    Angle,
    /// `[id]`, `[...slug]` — Next.js file-system routing.
    Bracket,
}

/// A best-effort type for a parameter. Absent is always acceptable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeHint {
    String,
    Integer,
    Number,
    Boolean,
    Uuid,
    /// Matches across `/` — Flask's `path`, Next.js catch-all.
    Path,
    Date,
    DateTime,
    Other(String),
}

impl TypeHint {
    /// Map a framework's converter name onto a hint. Unknown names are preserved rather than
    /// discarded, so nothing is silently lost.
    pub fn parse(raw: &str) -> TypeHint {
        match raw.trim().to_ascii_lowercase().as_str() {
            "str" | "string" => TypeHint::String,
            "int" | "integer" => TypeHint::Integer,
            "float" | "number" | "decimal" => TypeHint::Number,
            "bool" | "boolean" => TypeHint::Boolean,
            "uuid" => TypeHint::Uuid,
            "path" | "slug" => TypeHint::Path,
            "date" => TypeHint::Date,
            "datetime" => TypeHint::DateTime,
            other => TypeHint::Other(other.to_string()),
        }
    }
}

/// One `/`-delimited component of a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PathSegment {
    Literal {
        value: String,
    },
    Param {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ty: Option<TypeHint>,
        /// Matches the remainder of the path, across `/`.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        catch_all: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        optional: bool,
    },
    /// A segment that could not be determined statically.
    ///
    /// This exists so RouteLens never has to guess. `expr` holds the source text that
    /// defeated resolution, so the UI can show the developer exactly what to look at.
    Unresolved {
        expr: String,
    },
}

impl PathSegment {
    pub fn literal(value: impl Into<String>) -> Self {
        PathSegment::Literal {
            value: value.into(),
        }
    }

    pub fn param(name: impl Into<String>) -> Self {
        PathSegment::Param {
            name: name.into(),
            ty: None,
            catch_all: false,
            optional: false,
        }
    }

    pub fn unresolved(expr: impl Into<String>) -> Self {
        PathSegment::Unresolved { expr: expr.into() }
    }
}

/// A path shape: `/api/v1/users/{user_id}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct PathTemplate {
    pub segments: Vec<PathSegment>,
    /// Some frameworks treat `/users` and `/users/` as different routes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub trailing_slash: bool,
}

impl PathTemplate {
    pub fn empty() -> Self {
        PathTemplate::default()
    }

    pub fn from_segments(segments: Vec<PathSegment>) -> Self {
        PathTemplate {
            segments,
            trailing_slash: false,
        }
    }

    /// Parse a path written in a framework's own syntax.
    ///
    /// A segment is treated as a parameter only when the whole segment is one. A mixed
    /// segment such as `file-{id}.json` stays literal — rare enough that guessing at it
    /// would cost more than it gains.
    pub fn parse(raw: &str, style: ParamStyle) -> Self {
        let trimmed = raw.trim();
        let trailing_slash = trimmed.len() > 1 && trimmed.ends_with('/');

        let segments = trimmed
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| parse_segment(s, style))
            .collect();

        PathTemplate {
            segments,
            trailing_slash,
        }
    }

    /// Compose a prefix with a path relative to it.
    ///
    /// This is the operation the registration-graph resolver is built on: mounting a router
    /// at a prefix is exactly `prefix.join(route_path)`. Keeping it here — rather than in
    /// each framework adapter — is what keeps adapters thin.
    pub fn join(&self, other: &PathTemplate) -> PathTemplate {
        let mut segments = Vec::with_capacity(self.segments.len() + other.segments.len());
        segments.extend(self.segments.iter().cloned());
        segments.extend(other.segments.iter().cloned());

        PathTemplate {
            segments,
            // The child decides the trailing slash, unless it contributes nothing.
            trailing_slash: if other.segments.is_empty() {
                self.trailing_slash || other.trailing_slash
            } else {
                other.trailing_slash
            },
        }
    }

    /// Render in a framework's syntax.
    pub fn render(&self, style: ParamStyle) -> String {
        if self.segments.is_empty() {
            return "/".to_string();
        }

        let mut out = String::new();
        for segment in &self.segments {
            out.push('/');
            out.push_str(&render_segment(segment, style));
        }
        if self.trailing_slash {
            out.push('/');
        }
        out
    }

    /// Every segment was determined statically.
    pub fn is_resolved(&self) -> bool {
        !self
            .segments
            .iter()
            .any(|s| matches!(s, PathSegment::Unresolved { .. }))
    }

    /// The source expressions that defeated resolution, for display.
    pub fn unresolved_exprs(&self) -> Vec<&str> {
        self.segments
            .iter()
            .filter_map(|s| match s {
                PathSegment::Unresolved { expr } => Some(expr.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn param_names(&self) -> Vec<&str> {
        self.segments
            .iter()
            .filter_map(|s| match s {
                PathSegment::Param { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    /// A canonical form used to decide whether two endpoints are the same route.
    ///
    /// Parameter *names* are erased, because `/users/{id}` and `/users/{user_id}` describe
    /// the same route — the name is a local choice in the handler, not part of the API.
    /// Unresolved segments keep their expression so two different unknowns never collide.
    pub fn normalized(&self) -> String {
        if self.segments.is_empty() {
            return "/".to_string();
        }
        let mut out = String::new();
        for segment in &self.segments {
            out.push('/');
            match segment {
                PathSegment::Literal { value } => out.push_str(value),
                PathSegment::Param { catch_all, .. } => {
                    out.push_str(if *catch_all { "{*}" } else { "{}" })
                }
                PathSegment::Unresolved { expr } => {
                    out.push_str("{?");
                    out.push_str(expr);
                    out.push('}');
                }
            }
        }
        out
    }
}

impl fmt::Display for PathTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render(ParamStyle::Braces))
    }
}

fn parse_segment(raw: &str, style: ParamStyle) -> PathSegment {
    match style {
        ParamStyle::Braces => {
            if let Some(inner) = strip_wrapping(raw, '{', '}') {
                // Starlette/FastAPI converter syntax: `{user_id:int}`
                let (name, ty) = match inner.split_once(':') {
                    Some((n, t)) => (n.trim(), Some(TypeHint::parse(t))),
                    None => (inner.trim(), None),
                };
                let catch_all = matches!(ty, Some(TypeHint::Path));
                return PathSegment::Param {
                    name: name.to_string(),
                    ty,
                    catch_all,
                    optional: false,
                };
            }
        }
        ParamStyle::Colon => {
            if let Some(rest) = raw.strip_prefix(':') {
                let optional = rest.ends_with('?');
                let name = rest.trim_end_matches('?');
                return PathSegment::Param {
                    name: name.to_string(),
                    ty: None,
                    catch_all: false,
                    optional,
                };
            }
            if raw == "*" {
                return PathSegment::Param {
                    name: "wildcard".to_string(),
                    ty: Some(TypeHint::Path),
                    catch_all: true,
                    optional: false,
                };
            }
        }
        ParamStyle::Angle => {
            if let Some(inner) = strip_wrapping(raw, '<', '>') {
                // Werkzeug/Django converter syntax: `<int:user_id>`
                let (ty, name) = match inner.split_once(':') {
                    Some((t, n)) => (Some(TypeHint::parse(t)), n.trim()),
                    None => (None, inner.trim()),
                };
                let catch_all = matches!(ty, Some(TypeHint::Path));
                return PathSegment::Param {
                    name: name.to_string(),
                    ty,
                    catch_all,
                    optional: false,
                };
            }
        }
        ParamStyle::Bracket => {
            // `[[...slug]]` — optional catch-all
            if let Some(inner) =
                strip_wrapping(raw, '[', ']').and_then(|i| strip_wrapping(i, '[', ']'))
            {
                let name = inner.trim_start_matches("...");
                return PathSegment::Param {
                    name: name.to_string(),
                    ty: Some(TypeHint::Path),
                    catch_all: true,
                    optional: true,
                };
            }
            if let Some(inner) = strip_wrapping(raw, '[', ']') {
                let catch_all = inner.starts_with("...");
                let name = inner.trim_start_matches("...");
                return PathSegment::Param {
                    name: name.to_string(),
                    ty: if catch_all {
                        Some(TypeHint::Path)
                    } else {
                        None
                    },
                    catch_all,
                    optional: false,
                };
            }
        }
    }

    PathSegment::Literal {
        value: raw.to_string(),
    }
}

fn render_segment(segment: &PathSegment, style: ParamStyle) -> String {
    match segment {
        PathSegment::Literal { value } => value.clone(),
        PathSegment::Unresolved { .. } => "?".to_string(),
        PathSegment::Param {
            name,
            catch_all,
            optional,
            ..
        } => match style {
            ParamStyle::Braces => format!("{{{name}}}"),
            ParamStyle::Colon => {
                if *catch_all {
                    "*".to_string()
                } else if *optional {
                    format!(":{name}?")
                } else {
                    format!(":{name}")
                }
            }
            ParamStyle::Angle => {
                if *catch_all {
                    format!("<path:{name}>")
                } else {
                    format!("<{name}>")
                }
            }
            ParamStyle::Bracket => match (catch_all, optional) {
                (true, true) => format!("[[...{name}]]"),
                (true, false) => format!("[...{name}]"),
                _ => format!("[{name}]"),
            },
        },
    }
}

fn strip_wrapping(raw: &str, open: char, close: char) -> Option<&str> {
    let rest = raw.strip_prefix(open)?;
    rest.strip_suffix(close)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fastapi_style() {
        let p = PathTemplate::parse("/users/{user_id}", ParamStyle::Braces);
        assert_eq!(p.segments.len(), 2);
        assert_eq!(p.param_names(), vec!["user_id"]);
        assert_eq!(p.render(ParamStyle::Braces), "/users/{user_id}");
    }

    #[test]
    fn parses_starlette_converter() {
        let p = PathTemplate::parse("/users/{user_id:int}", ParamStyle::Braces);
        match &p.segments[1] {
            PathSegment::Param { name, ty, .. } => {
                assert_eq!(name, "user_id");
                assert_eq!(ty.as_ref(), Some(&TypeHint::Integer));
            }
            other => panic!("expected param, got {other:?}"),
        }
    }

    #[test]
    fn parses_express_style() {
        let p = PathTemplate::parse("/users/:id", ParamStyle::Colon);
        assert_eq!(p.param_names(), vec!["id"]);

        let opt = PathTemplate::parse("/users/:id?", ParamStyle::Colon);
        match &opt.segments[1] {
            PathSegment::Param { optional, .. } => assert!(optional),
            other => panic!("expected param, got {other:?}"),
        }
    }

    #[test]
    fn parses_flask_converter() {
        let p = PathTemplate::parse("/files/<path:subpath>", ParamStyle::Angle);
        match &p.segments[1] {
            PathSegment::Param {
                name,
                ty,
                catch_all,
                ..
            } => {
                assert_eq!(name, "subpath");
                assert_eq!(ty.as_ref(), Some(&TypeHint::Path));
                assert!(catch_all);
            }
            other => panic!("expected param, got {other:?}"),
        }
    }

    #[test]
    fn parses_nextjs_segments() {
        let dynamic = PathTemplate::parse("/api/users/[id]", ParamStyle::Bracket);
        assert_eq!(dynamic.param_names(), vec!["id"]);

        let catch = PathTemplate::parse("/api/[...slug]", ParamStyle::Bracket);
        match &catch.segments[1] {
            PathSegment::Param {
                name, catch_all, ..
            } => {
                assert_eq!(name, "slug");
                assert!(catch_all);
            }
            other => panic!("expected param, got {other:?}"),
        }

        let optional = PathTemplate::parse("/api/[[...slug]]", ParamStyle::Bracket);
        match &optional.segments[1] {
            PathSegment::Param {
                catch_all,
                optional,
                ..
            } => {
                assert!(catch_all);
                assert!(optional);
            }
            other => panic!("expected param, got {other:?}"),
        }
    }

    #[test]
    fn mixed_segments_stay_literal() {
        let p = PathTemplate::parse("/files/report-{id}.json", ParamStyle::Braces);
        assert!(matches!(p.segments[1], PathSegment::Literal { .. }));
    }

    #[test]
    fn join_composes_router_prefixes() {
        // The FastAPI case: APIRouter(prefix="/users") included at prefix="/api/v1".
        let mount = PathTemplate::parse("/api/v1", ParamStyle::Braces);
        let router = PathTemplate::parse("/users", ParamStyle::Braces);
        let route = PathTemplate::parse("/{user_id}", ParamStyle::Braces);

        let full = mount.join(&router).join(&route);
        assert_eq!(full.render(ParamStyle::Braces), "/api/v1/users/{user_id}");
    }

    #[test]
    fn join_with_empty_is_identity() {
        let base = PathTemplate::parse("/users", ParamStyle::Braces);
        assert_eq!(base.join(&PathTemplate::empty()), base);
        assert_eq!(PathTemplate::empty().join(&base), base);
    }

    #[test]
    fn empty_template_renders_as_root() {
        assert_eq!(PathTemplate::empty().render(ParamStyle::Braces), "/");
        assert_eq!(PathTemplate::empty().normalized(), "/");
    }

    #[test]
    fn trailing_slash_is_preserved() {
        let p = PathTemplate::parse("/users/", ParamStyle::Braces);
        assert!(p.trailing_slash);
        assert_eq!(p.render(ParamStyle::Braces), "/users/");
    }

    #[test]
    fn unresolved_segments_are_visible_not_guessed() {
        let template = PathTemplate::from_segments(vec![
            PathSegment::unresolved("settings.API_PREFIX"),
            PathSegment::literal("users"),
        ]);
        assert!(!template.is_resolved());
        assert_eq!(template.unresolved_exprs(), vec!["settings.API_PREFIX"]);
        assert_eq!(template.render(ParamStyle::Braces), "/?/users");
    }

    #[test]
    fn normalization_erases_param_names_but_not_unknowns() {
        let a = PathTemplate::parse("/users/{id}", ParamStyle::Braces);
        let b = PathTemplate::parse("/users/:user_id", ParamStyle::Colon);
        assert_eq!(a.normalized(), b.normalized());

        let x = PathTemplate::from_segments(vec![PathSegment::unresolved("A")]);
        let y = PathTemplate::from_segments(vec![PathSegment::unresolved("B")]);
        assert_ne!(x.normalized(), y.normalized());
    }

    #[test]
    fn catch_all_normalizes_distinctly_from_a_plain_param() {
        let plain = PathTemplate::parse("/api/[id]", ParamStyle::Bracket);
        let catch = PathTemplate::parse("/api/[...id]", ParamStyle::Bracket);
        assert_ne!(plain.normalized(), catch.normalized());
    }

    #[test]
    fn cross_style_rendering_round_trips() {
        let p = PathTemplate::parse("/users/{id}", ParamStyle::Braces);
        assert_eq!(p.render(ParamStyle::Colon), "/users/:id");
        assert_eq!(p.render(ParamStyle::Bracket), "/users/[id]");
        assert_eq!(p.render(ParamStyle::Angle), "/users/<id>");
    }
}
