//! The route registration graph.
//!
//! Every framework except a file-system-routed one has the same shape: a router declared in
//! one file, mounted with a prefix in another. The real path exists in neither file alone.
//!
//! ```text
//! Module ──declares──▶ RouterSymbol ──has──▶ RouteRegistration
//!                           │
//!                           └──mounted at "/api/v1"──▶ RouterSymbol (app root)
//! ```
//!
//! Composing those prefixes is written here, once, rather than in each adapter. That is what
//! keeps adapters small enough to add cheaply.

use crate::facts::{FactSink, ImportFact, MountFact, RouteFact, RouterFact, SymbolId, SymbolRef};
use rl_model::PathTemplate;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Guards against a mount chain that never terminates.
const MAX_DEPTH: usize = 32;

/// Something the graph could not do, worth telling the developer about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphWarning {
    /// A `include_router(x)` whose `x` could not be traced to a declaration.
    UnresolvedReference { module: PathBuf, name: String },
    /// A router was declared but never mounted, so its routes may be unreachable.
    ///
    /// Kept and reported rather than dropped: this is usually a bug in the project being
    /// inspected, and saying so is more useful than staying silent.
    OrphanedRouter { symbol: String },
    /// Mount edges formed a loop.
    Cycle { symbol: String },
    /// Resolution stopped at [`MAX_DEPTH`].
    TooDeep { symbol: String },
}

impl std::fmt::Display for GraphWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphWarning::UnresolvedReference { module, name } => write!(
                f,
                "could not trace `{name}` in {} to a router declaration",
                module.display()
            ),
            GraphWarning::OrphanedRouter { symbol } => {
                write!(
                    f,
                    "router `{symbol}` is never mounted, so its routes may be unreachable"
                )
            }
            GraphWarning::Cycle { symbol } => {
                write!(f, "mount cycle involving `{symbol}`")
            }
            GraphWarning::TooDeep { symbol } => {
                write!(f, "mount chain from `{symbol}` is deeper than {MAX_DEPTH}")
            }
        }
    }
}

/// One route, with its path fully composed.
#[derive(Debug, Clone)]
pub struct ResolvedRoute<'a> {
    pub method: rl_model::HttpMethod,
    /// Mount prefixes, router prefix, and route path, joined.
    pub path: PathTemplate,
    pub fact: &'a RouteFact,
    /// Nearest group: the mount's, then the router's, then the route's own.
    pub group: Option<String>,
    /// Reached from no application root.
    pub orphaned: bool,
}

/// Routes and everything the graph could not do.
#[derive(Debug)]
pub struct Resolution<'a> {
    pub routes: Vec<ResolvedRoute<'a>>,
    pub warnings: Vec<GraphWarning>,
}

/// The assembled graph.
#[derive(Debug, Default)]
pub struct RegistrationGraph {
    routers: BTreeMap<SymbolId, RouterFact>,
    routes: Vec<RouteFact>,
    mounts: Vec<MountFact>,
    /// (module, local name) → what it refers to.
    import_bindings: BTreeMap<(PathBuf, String), Binding>,
}

/// What an imported name points at.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Binding {
    /// `from .users import router` — a name inside another module.
    Symbol(SymbolId),
    /// `import api.users as users` — the module itself, so `users.router` can be resolved.
    Module(PathBuf),
}

impl RegistrationGraph {
    /// Build the graph, resolving imports against the set of files actually in the project.
    pub fn build(sink: FactSink, project_files: &BTreeSet<PathBuf>) -> RegistrationGraph {
        let known = |path: &Path| project_files.contains(path);

        let mut routers = BTreeMap::new();
        for fact in sink.routers {
            // A module may re-run a declaration (a factory called twice); the first wins so
            // the recorded source location is the declaration site.
            routers.entry(fact.symbol.clone()).or_insert(fact);
        }

        let import_bindings = resolve_imports(&sink.imports, &known);

        RegistrationGraph {
            routers,
            routes: sink.routes,
            mounts: sink.mounts,
            import_bindings,
        }
    }

    pub fn router_count(&self) -> usize {
        self.routers.len()
    }

    /// Trace a reference back to the symbol it names.
    fn resolve_symbol(&self, reference: &SymbolRef) -> Option<SymbolId> {
        let (qualifier, attribute) = reference.split();

        match qualifier {
            // `users.router`: the qualifier names a module, so the attribute lives there.
            Some(qualifier) => {
                let key = (reference.module.clone(), qualifier.to_string());
                match self.import_bindings.get(&key) {
                    Some(Binding::Module(module)) => Some(SymbolId::new(module.clone(), attribute)),
                    // A qualifier bound to a *value* rather than a module — `from .models
                    // import user` then `user.router`. Reading an attribute off a value
                    // needs evaluation, so this is reported rather than guessed at.
                    Some(Binding::Symbol(_)) => None,
                    None => None,
                }
            }

            None => {
                // Declared right here?
                let local = SymbolId::new(reference.module.clone(), attribute);
                if self.routers.contains_key(&local) {
                    return Some(local);
                }
                // Imported by name?
                let key = (reference.module.clone(), attribute.to_string());
                match self.import_bindings.get(&key) {
                    Some(Binding::Symbol(symbol)) => Some(symbol.clone()),
                    Some(Binding::Module(_)) => None,
                    // Not declared and not imported. Still return the local id: an app root
                    // such as `app = FastAPI()` is a symbol we know about even when no
                    // RouterFact was recorded for it.
                    None => Some(local),
                }
            }
        }
    }

    /// Compose every route's full path.
    ///
    /// Walks from each application root, then sweeps up whatever was never reached.
    pub fn resolve(&self) -> Resolution<'_> {
        let mut warnings = Vec::new();
        // Index routes and mounts by the symbol they attach to, so the walk is linear.
        let mut routes_by_router: BTreeMap<SymbolId, Vec<usize>> = BTreeMap::new();
        for (index, route) in self.routes.iter().enumerate() {
            match self.resolve_symbol(&route.router) {
                Some(symbol) => routes_by_router.entry(symbol).or_default().push(index),
                None => warnings.push(GraphWarning::UnresolvedReference {
                    module: route.router.module.clone(),
                    name: route.router.name.clone(),
                }),
            }
        }

        let mut mounts_by_parent: BTreeMap<SymbolId, Vec<(SymbolId, &MountFact)>> = BTreeMap::new();
        for mount in &self.mounts {
            let (Some(parent), Some(child)) = (
                self.resolve_symbol(&mount.parent),
                self.resolve_symbol(&mount.child),
            ) else {
                warnings.push(GraphWarning::UnresolvedReference {
                    module: mount.child.module.clone(),
                    name: mount.child.name.clone(),
                });
                continue;
            };
            mounts_by_parent
                .entry(parent)
                .or_default()
                .push((child, mount));
        }

        let roots: Vec<SymbolId> = self
            .routers
            .values()
            .filter(|r| r.is_app_root)
            .map(|r| r.symbol.clone())
            .collect();

        let mut resolved = Vec::new();
        let mut reached: BTreeSet<SymbolId> = BTreeSet::new();

        for root in &roots {
            let mut stack = Vec::new();
            self.walk(
                root,
                &PathTemplate::empty(),
                None,
                &routes_by_router,
                &mounts_by_parent,
                &mut stack,
                &mut reached,
                &mut resolved,
                &mut warnings,
                false,
            );
        }

        // Anything never reached from a root. Reported, not dropped.
        let unreached: Vec<SymbolId> = self
            .routers
            .keys()
            .filter(|symbol| !reached.contains(*symbol))
            .cloned()
            .collect();

        for symbol in unreached {
            warnings.push(GraphWarning::OrphanedRouter {
                symbol: symbol.to_string(),
            });
            let mut stack = Vec::new();
            let mut orphan_reached = BTreeSet::new();
            self.walk(
                &symbol,
                &PathTemplate::empty(),
                None,
                &routes_by_router,
                &mounts_by_parent,
                &mut stack,
                &mut orphan_reached,
                &mut resolved,
                &mut warnings,
                true,
            );
        }

        Resolution {
            routes: resolved,
            warnings,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn walk<'a>(
        &'a self,
        symbol: &SymbolId,
        prefix: &PathTemplate,
        group: Option<&str>,
        routes_by_router: &BTreeMap<SymbolId, Vec<usize>>,
        mounts_by_parent: &BTreeMap<SymbolId, Vec<(SymbolId, &MountFact)>>,
        stack: &mut Vec<SymbolId>,
        reached: &mut BTreeSet<SymbolId>,
        out: &mut Vec<ResolvedRoute<'a>>,
        warnings: &mut Vec<GraphWarning>,
        orphaned: bool,
    ) {
        // The stack, not a global visited set: a router legitimately mounted at two prefixes
        // must be walked twice and produce both paths.
        if stack.contains(symbol) {
            warnings.push(GraphWarning::Cycle {
                symbol: symbol.to_string(),
            });
            return;
        }
        if stack.len() >= MAX_DEPTH {
            warnings.push(GraphWarning::TooDeep {
                symbol: symbol.to_string(),
            });
            return;
        }

        reached.insert(symbol.clone());
        stack.push(symbol.clone());

        let declared = self.routers.get(symbol);
        let own_prefix = declared.map(|r| r.prefix.clone()).unwrap_or_default();
        let base = prefix.join(&own_prefix);
        let group = declared.and_then(|r| r.group.as_deref()).or(group);

        for index in routes_by_router.get(symbol).into_iter().flatten() {
            let fact = &self.routes[*index];
            let full = base.join(&fact.path);
            for method in &fact.methods {
                out.push(ResolvedRoute {
                    method: method.clone(),
                    path: full.clone(),
                    fact,
                    group: fact.group.clone().or_else(|| group.map(str::to_string)),
                    orphaned,
                });
            }
        }

        for (child, mount) in mounts_by_parent.get(symbol).into_iter().flatten() {
            let mounted_at = base.join(&mount.prefix);
            let child_group = mount.group.as_deref().or(group);
            self.walk(
                child,
                &mounted_at,
                child_group,
                routes_by_router,
                mounts_by_parent,
                stack,
                reached,
                out,
                warnings,
                orphaned,
            );
        }

        stack.pop();
    }
}

/// Turn import statements into name bindings.
fn resolve_imports(
    imports: &[ImportFact],
    known: &dyn Fn(&Path) -> bool,
) -> BTreeMap<(PathBuf, String), Binding> {
    let mut bindings = BTreeMap::new();

    for import in imports {
        let Some(module) = crate::facts::resolve_python_module(
            &import.module,
            &import.source,
            import.level,
            known,
        ) else {
            // An import of something outside the project — `fastapi` itself, say. Not an
            // error; there is simply nothing in the project to link it to.
            continue;
        };

        let binding = match &import.original {
            Some(name) => {
                // `from .api import admin` looks like a symbol import but usually names a
                // *submodule*, so `admin.router` means `api/admin.py`'s `router`. Try that
                // first; fall back to a symbol when no such file exists.
                let nested = if import.source.is_empty() {
                    name.clone()
                } else {
                    format!("{}.{}", import.source, name)
                };
                match crate::facts::resolve_python_module(
                    &import.module,
                    &nested,
                    import.level,
                    known,
                ) {
                    Some(submodule) => Binding::Module(submodule),
                    None => Binding::Symbol(SymbolId::new(module, name)),
                }
            }
            None => Binding::Module(module),
        };

        bindings.insert((import.module.clone(), import.local_name.clone()), binding);
    }

    bindings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{MountFact, RouteFact, RouterFact, Span};
    use rl_model::{HttpMethod, ParamStyle};

    fn files(paths: &[&str]) -> BTreeSet<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    fn path(raw: &str) -> PathTemplate {
        PathTemplate::parse(raw, ParamStyle::Braces)
    }

    fn app_root(module: &str, name: &str) -> RouterFact {
        RouterFact {
            symbol: SymbolId::new(module, name),
            prefix: PathTemplate::empty(),
            group: None,
            is_app_root: true,
            span: Span::new(1, 1),
        }
    }

    fn router(module: &str, name: &str, prefix: &str, group: Option<&str>) -> RouterFact {
        RouterFact {
            symbol: SymbolId::new(module, name),
            prefix: path(prefix),
            group: group.map(str::to_string),
            is_app_root: false,
            span: Span::new(1, 1),
        }
    }

    fn route(module: &str, router: &str, method: HttpMethod, p: &str) -> RouteFact {
        RouteFact::new(
            SymbolRef::new(module, router),
            method,
            path(p),
            Span::new(1, 1),
        )
    }

    fn mount(module: &str, parent: &str, child: &str, prefix: &str) -> MountFact {
        MountFact {
            parent: SymbolRef::new(module, parent),
            child: SymbolRef::new(module, child),
            prefix: path(prefix),
            group: None,
            span: Span::new(1, 1),
        }
    }

    fn rendered(routes: &[ResolvedRoute<'_>]) -> Vec<String> {
        let mut out: Vec<String> = routes
            .iter()
            .map(|r| format!("{} {}", r.method, r.path.render(ParamStyle::Braces)))
            .collect();
        out.sort();
        out
    }

    /// The shape from `docs/discovery/how-it-works.md`: a router with its own prefix,
    /// declared in one file, mounted at another prefix in a second file.
    #[test]
    fn composes_a_prefix_across_two_files() {
        let mut sink = FactSink::new();
        sink.router(app_root("main.py", "app"));
        sink.router(router("api/users.py", "router", "/users", Some("users")));
        sink.route(route(
            "api/users.py",
            "router",
            HttpMethod::Get,
            "/{user_id}",
        ));
        sink.import(ImportFact {
            module: PathBuf::from("main.py"),
            local_name: "router".into(),
            source: "api.users".into(),
            original: Some("router".into()),
            level: 0,
        });
        sink.mount(mount("main.py", "app", "router", "/api/v1"));

        let graph = RegistrationGraph::build(sink, &files(&["main.py", "api/users.py"]));
        let resolution = graph.resolve();
        let routes = &resolution.routes;

        assert_eq!(rendered(routes), vec!["GET /api/v1/users/{user_id}"]);
        assert_eq!(routes[0].group.as_deref(), Some("users"));
        assert!(!routes[0].orphaned);
    }

    #[test]
    fn a_dotted_reference_resolves_through_a_module_import() {
        let mut sink = FactSink::new();
        sink.router(app_root("main.py", "app"));
        sink.router(router("api/users.py", "router", "/users", None));
        sink.route(route("api/users.py", "router", HttpMethod::Get, "/"));
        // `from api import users` then `users.router`
        sink.import(ImportFact {
            module: PathBuf::from("main.py"),
            local_name: "users".into(),
            source: "api.users".into(),
            original: None,
            level: 0,
        });
        sink.mount(mount("main.py", "app", "users.router", "/api"));

        let graph = RegistrationGraph::build(sink, &files(&["main.py", "api/users.py"]));
        assert_eq!(rendered(&graph.resolve().routes), vec!["GET /api/users"]);
    }

    #[test]
    fn a_router_mounted_twice_produces_both_paths() {
        let mut sink = FactSink::new();
        sink.router(app_root("main.py", "app"));
        sink.router(router("api/items.py", "router", "", None));
        sink.route(route("api/items.py", "router", HttpMethod::Get, "/items"));
        sink.import(ImportFact {
            module: PathBuf::from("main.py"),
            local_name: "router".into(),
            source: "api.items".into(),
            original: Some("router".into()),
            level: 0,
        });
        sink.mount(mount("main.py", "app", "router", "/v1"));
        sink.mount(mount("main.py", "app", "router", "/v2"));

        let graph = RegistrationGraph::build(sink, &files(&["main.py", "api/items.py"]));
        assert_eq!(
            rendered(&graph.resolve().routes),
            vec!["GET /v1/items", "GET /v2/items"]
        );
    }

    #[test]
    fn nested_mounts_compose_in_order() {
        let mut sink = FactSink::new();
        sink.router(app_root("main.py", "app"));
        sink.router(router("api/v1.py", "v1", "/v1", None));
        sink.router(router("api/users.py", "users", "/users", None));
        sink.route(route("api/users.py", "users", HttpMethod::Get, "/{id}"));

        sink.import(ImportFact {
            module: PathBuf::from("main.py"),
            local_name: "v1".into(),
            source: "api.v1".into(),
            original: Some("v1".into()),
            level: 0,
        });
        sink.import(ImportFact {
            module: PathBuf::from("api/v1.py"),
            local_name: "users".into(),
            source: "users".into(),
            original: Some("users".into()),
            level: 1,
        });
        sink.mount(mount("main.py", "app", "v1", "/api"));
        sink.mount(mount("api/v1.py", "v1", "users", ""));

        let graph =
            RegistrationGraph::build(sink, &files(&["main.py", "api/v1.py", "api/users.py"]));
        assert_eq!(
            rendered(&graph.resolve().routes),
            vec!["GET /api/v1/users/{id}"]
        );
    }

    #[test]
    fn an_unmounted_router_is_reported_and_its_routes_kept() {
        let mut sink = FactSink::new();
        sink.router(app_root("main.py", "app"));
        sink.router(router("api/orphan.py", "router", "/orphan", None));
        sink.route(route("api/orphan.py", "router", HttpMethod::Get, "/x"));

        let graph = RegistrationGraph::build(sink, &files(&["main.py", "api/orphan.py"]));
        let resolution = graph.resolve();
        let routes = &resolution.routes;

        assert_eq!(rendered(routes), vec!["GET /orphan/x"]);
        assert!(routes[0].orphaned, "an unmounted route must be flagged");
        assert!(resolution
            .warnings
            .iter()
            .any(|w| matches!(w, GraphWarning::OrphanedRouter { .. })));
    }

    #[test]
    fn a_mount_cycle_is_broken_and_reported() {
        let mut sink = FactSink::new();
        sink.router(app_root("main.py", "app"));
        sink.router(router("a.py", "a", "/a", None));
        sink.router(router("b.py", "b", "/b", None));
        sink.route(route("a.py", "a", HttpMethod::Get, "/x"));

        for (module, local, source) in [
            ("main.py", "a", "a"),
            ("a.py", "b", "b"),
            ("b.py", "a", "a"),
        ] {
            sink.import(ImportFact {
                module: PathBuf::from(module),
                local_name: local.into(),
                source: source.into(),
                original: Some(local.into()),
                level: 0,
            });
        }
        sink.mount(mount("main.py", "app", "a", ""));
        sink.mount(mount("a.py", "a", "b", ""));
        sink.mount(mount("b.py", "b", "a", "")); // back to a

        let graph = RegistrationGraph::build(sink, &files(&["main.py", "a.py", "b.py"]));
        let resolution = graph.resolve();
        let routes = &resolution.routes;

        // Terminated, and the route was still found on the way round.
        assert!(rendered(routes).contains(&"GET /a/x".to_string()));
        assert!(resolution
            .warnings
            .iter()
            .any(|w| matches!(w, GraphWarning::Cycle { .. })));
    }

    #[test]
    fn a_route_registered_directly_on_the_app_needs_no_router() {
        let mut sink = FactSink::new();
        sink.router(app_root("main.py", "app"));
        sink.route(route("main.py", "app", HttpMethod::Get, "/health"));

        let graph = RegistrationGraph::build(sink, &files(&["main.py"]));
        assert_eq!(rendered(&graph.resolve().routes), vec!["GET /health"]);
    }

    #[test]
    fn one_registration_with_several_methods_becomes_several_routes() {
        let mut sink = FactSink::new();
        sink.router(app_root("main.py", "app"));
        let mut multi = route("main.py", "app", HttpMethod::Get, "/items");
        multi.methods = vec![HttpMethod::Get, HttpMethod::Post];
        sink.route(multi);

        let graph = RegistrationGraph::build(sink, &files(&["main.py"]));
        assert_eq!(
            rendered(&graph.resolve().routes),
            vec!["GET /items", "POST /items"]
        );
    }

    #[test]
    fn an_unresolvable_mount_target_is_reported() {
        let mut sink = FactSink::new();
        sink.router(app_root("main.py", "app"));
        // `ghost` is neither declared here nor imported from anywhere in the project.
        sink.mount(mount("main.py", "app", "ghost.router", "/x"));

        let graph = RegistrationGraph::build(sink, &files(&["main.py"]));
        let resolution = graph.resolve();

        assert!(resolution
            .warnings
            .iter()
            .any(|w| matches!(w, GraphWarning::UnresolvedReference { .. })));
    }

    #[test]
    fn a_relative_import_links_modules() {
        let mut sink = FactSink::new();
        sink.router(app_root("app/main.py", "app"));
        sink.router(router("app/api/users.py", "router", "/users", None));
        sink.route(route("app/api/users.py", "router", HttpMethod::Get, "/"));
        // from .api.users import router
        sink.import(ImportFact {
            module: PathBuf::from("app/main.py"),
            local_name: "router".into(),
            source: "api.users".into(),
            original: Some("router".into()),
            level: 1,
        });
        sink.mount(mount("app/main.py", "app", "router", ""));

        let graph = RegistrationGraph::build(sink, &files(&["app/main.py", "app/api/users.py"]));
        assert_eq!(rendered(&graph.resolve().routes), vec!["GET /users"]);
    }
}
