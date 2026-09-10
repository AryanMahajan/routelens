//! What a framework adapter emits.
//!
//! Adapters recognise three things — *this creates a router*, *this registers a route*, *this
//! mounts a router at a prefix* — plus the imports needed to link them across files. They do
//! **not** compose paths. [`crate::graph`] does that, once, for every framework.
//!
//! If an adapter ever starts joining paths itself, the abstraction has sprung a leak.

use rl_model::{AuthRequirement, BodySchema, HttpMethod, ParamSpec, PathTemplate};
use std::path::{Path, PathBuf};

/// A position in a source file, 1-indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: u32,
    pub column: u32,
}

impl Span {
    pub fn new(line: u32, column: u32) -> Self {
        Span { line, column }
    }
}

/// A symbol declared in a module — normally a router variable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolId {
    /// Relative to the project root.
    pub module: PathBuf,
    pub name: String,
}

impl SymbolId {
    pub fn new(module: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        SymbolId {
            module: module.into(),
            name: name.into(),
        }
    }
}

impl std::fmt::Display for SymbolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.module.display(), self.name)
    }
}

/// A *use* of a symbol, from inside some module.
///
/// Unresolved on purpose: `include_router(router)` in `main.py` might mean a local variable
/// or something imported from another file, and only the graph knows which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRef {
    /// The module the reference appears in.
    pub module: PathBuf,
    /// As written — possibly dotted, e.g. `users.router`.
    pub name: String,
}

impl SymbolRef {
    pub fn new(module: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        SymbolRef {
            module: module.into(),
            name: name.into(),
        }
    }

    /// Split a dotted reference into its qualifier and final attribute.
    ///
    /// `users.router` → (`Some("users")`, `"router"`); `router` → (`None`, `"router"`).
    pub fn split(&self) -> (Option<&str>, &str) {
        match self.name.rsplit_once('.') {
            Some((qualifier, attribute)) => (Some(qualifier), attribute),
            None => (None, self.name.as_str()),
        }
    }
}

/// A router was created.
#[derive(Debug, Clone)]
pub struct RouterFact {
    pub symbol: SymbolId,
    /// The prefix the router carries itself, e.g. `APIRouter(prefix="/users")`.
    pub prefix: PathTemplate,
    /// Tags or a name, used to group endpoints in the explorer.
    pub group: Option<String>,
    /// This symbol is an application root, so path resolution starts here.
    pub is_app_root: bool,
    pub span: Span,
}

/// A route was registered on a router.
#[derive(Debug, Clone)]
pub struct RouteFact {
    pub router: SymbolRef,
    /// One registration may declare several methods, as `@app.route(methods=[...])` does.
    pub methods: Vec<HttpMethod>,
    /// Relative to the router it is registered on.
    pub path: PathTemplate,
    pub query_params: Vec<ParamSpec>,
    pub headers: Vec<ParamSpec>,
    pub body: Option<BodySchema>,
    pub auth: Option<AuthRequirement>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub group: Option<String>,
    pub deprecated: bool,
    pub span: Span,
}

impl RouteFact {
    pub fn new(router: SymbolRef, method: HttpMethod, path: PathTemplate, span: Span) -> Self {
        RouteFact {
            router,
            methods: vec![method],
            path,
            query_params: Vec::new(),
            headers: Vec::new(),
            body: None,
            auth: None,
            summary: None,
            description: None,
            group: None,
            deprecated: false,
            span,
        }
    }
}

/// A router was mounted onto another router at a prefix.
#[derive(Debug, Clone)]
pub struct MountFact {
    pub parent: SymbolRef,
    pub child: SymbolRef,
    pub prefix: PathTemplate,
    pub group: Option<String>,
    pub span: Span,
}

/// A name was bound by an import.
///
/// Needed because the router declared in `api/users.py` and the `include_router` call in
/// `main.py` are only connected by an import statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportFact {
    /// The importing module.
    pub module: PathBuf,
    /// The name bound locally.
    pub local_name: String,
    /// The module path as written, without leading dots: `api.users`, or `` for `from . import x`.
    pub source: String,
    /// The name inside the source module, or `None` when the module itself was bound
    /// (`import api.users as users`).
    pub original: Option<String>,
    /// Leading dots on a relative import. `0` means absolute.
    pub level: u32,
}

/// Where adapters push what they found.
#[derive(Debug, Default)]
pub struct FactSink {
    pub routers: Vec<RouterFact>,
    pub routes: Vec<RouteFact>,
    pub mounts: Vec<MountFact>,
    pub imports: Vec<ImportFact>,
    /// Anything the adapter noticed but could not express.
    pub warnings: Vec<String>,
}

impl FactSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn router(&mut self, fact: RouterFact) {
        self.routers.push(fact);
    }

    pub fn route(&mut self, fact: RouteFact) {
        self.routes.push(fact);
    }

    pub fn mount(&mut self, fact: MountFact) {
        self.mounts.push(fact);
    }

    pub fn import(&mut self, fact: ImportFact) {
        self.imports.push(fact);
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    pub fn is_empty(&self) -> bool {
        self.routers.is_empty() && self.routes.is_empty() && self.mounts.is_empty()
    }

    pub fn absorb(&mut self, other: FactSink) {
        self.routers.extend(other.routers);
        self.routes.extend(other.routes);
        self.mounts.extend(other.mounts);
        self.imports.extend(other.imports);
        self.warnings.extend(other.warnings);
    }
}

/// Resolve a Python-style module specification to a file in the project.
///
/// Handles the relative form (`from .users import router`, `from ..core import settings`) and
/// the absolute form (`from app.api.users import router`), trying both `<path>.py` and
/// `<path>/__init__.py`.
pub fn resolve_python_module(
    importing: &Path,
    source: &str,
    level: u32,
    known: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let mut base = if level == 0 {
        // Absolute: from the project root. A `src/` or package-dir layout means the same
        // module may live one level down, so both are tried below.
        PathBuf::new()
    } else {
        // Relative: one dot means the importing module's own package, each extra dot climbs.
        let mut dir = importing.parent()?.to_path_buf();
        for _ in 1..level {
            dir = dir.parent()?.to_path_buf();
        }
        dir
    };

    for segment in source.split('.').filter(|s| !s.is_empty()) {
        base = base.join(segment);
    }

    let candidates = [base.with_extension("py"), base.join("__init__.py")];
    if let Some(hit) = candidates.iter().find(|c| known(c)) {
        return Some(hit.clone());
    }

    // An absolute import may still be rooted inside a package directory, so try the
    // importing module's own top-level directory as a base before giving up.
    if level == 0 {
        let top = importing.components().next()?;
        let mut nested = PathBuf::from(top.as_os_str());
        for segment in source.split('.').filter(|s| !s.is_empty()) {
            nested = nested.join(segment);
        }
        let candidates = [nested.with_extension("py"), nested.join("__init__.py")];
        if let Some(hit) = candidates.iter().find(|c| known(c)) {
            return Some(hit.clone());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn known_files(paths: &[&str]) -> impl Fn(&Path) -> bool {
        // Owns the set, so the closure borrows nothing and needs no lifetime.
        let set: BTreeSet<PathBuf> = paths.iter().map(PathBuf::from).collect();
        move |p: &Path| set.contains(p)
    }

    #[test]
    fn a_dotted_reference_splits_into_qualifier_and_attribute() {
        let dotted = SymbolRef::new("main.py", "users.router");
        assert_eq!(dotted.split(), (Some("users"), "router"));

        let plain = SymbolRef::new("main.py", "router");
        assert_eq!(plain.split(), (None, "router"));
    }

    #[test]
    fn a_deeply_dotted_reference_keeps_the_whole_qualifier() {
        let deep = SymbolRef::new("main.py", "api.v1.users.router");
        assert_eq!(deep.split(), (Some("api.v1.users"), "router"));
    }

    #[test]
    fn a_relative_import_resolves_against_the_importing_package() {
        let known = known_files(&["app/api/users.py"]);
        assert_eq!(
            resolve_python_module(Path::new("app/api/__init__.py"), "users", 1, &known),
            Some(PathBuf::from("app/api/users.py"))
        );
    }

    #[test]
    fn extra_dots_climb_out_of_the_package() {
        let known = known_files(&["app/core/config.py"]);
        assert_eq!(
            resolve_python_module(Path::new("app/api/users.py"), "core.config", 2, &known),
            Some(PathBuf::from("app/core/config.py"))
        );
    }

    #[test]
    fn an_absolute_import_resolves_from_the_project_root() {
        let known = known_files(&["app/api/users.py"]);
        assert_eq!(
            resolve_python_module(Path::new("main.py"), "app.api.users", 0, &known),
            Some(PathBuf::from("app/api/users.py"))
        );
    }

    #[test]
    fn a_package_is_found_through_its_init_file() {
        let known = known_files(&["app/api/__init__.py"]);
        assert_eq!(
            resolve_python_module(Path::new("main.py"), "app.api", 0, &known),
            Some(PathBuf::from("app/api/__init__.py"))
        );
    }

    /// A project laid out as `src/app/...` imports itself as `app.api.users`, so the
    /// top-level directory has to be tried as a base too.
    #[test]
    fn an_absolute_import_inside_a_package_directory_resolves() {
        let known = known_files(&["src/app/api/users.py"]);
        assert_eq!(
            resolve_python_module(Path::new("src/app/main.py"), "app.api.users", 0, &known),
            Some(PathBuf::from("src/app/api/users.py"))
        );
    }

    #[test]
    fn an_import_of_something_outside_the_project_does_not_resolve() {
        let known = known_files(&["app/main.py"]);
        assert_eq!(
            resolve_python_module(Path::new("app/main.py"), "fastapi", 0, &known),
            None
        );
    }

    #[test]
    fn the_sink_absorbs_another() {
        let mut a = FactSink::new();
        a.warn("one");
        let mut b = FactSink::new();
        b.warn("two");

        a.absorb(b);
        assert_eq!(a.warnings, vec!["one", "two"]);
        assert!(a.is_empty(), "warnings alone are not facts");
    }
}
