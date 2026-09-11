//! Django and Django REST Framework.
//!
//! Django's routing is a tree of `urlpatterns` lists joined by `include()`, rooted at the
//! module `ROOT_URLCONF` names in settings. That maps onto the graph directly: every
//! pattern list is a router, `include()` is a mount, and the settings file's `ROOT_URLCONF`
//! is the application root with one mount onto the root URL conf.
//!
//! Views are where Django differs from the other frameworks: a URL pattern names a view
//! that lives in another file, and only the view knows which methods it serves. So every
//! view — function or class — is a router of its own, with one empty-path route per method,
//! and `path("users/", views.list_users)` is a *mount* of that view. The graph links the
//! two through the import, exactly as it links a blueprint to its registration. Views are
//! declared *implicitly*: a function nobody routes to is not reported as an orphan, because
//! it is just a function.
//!
//! DRF's `router.register("users", UserViewSet)` is the same shape one level up: the
//! ViewSet is a router whose routes are the actions it implements — `list`, `create`,
//! `retrieve`, … and `@action`s — mounted at the registered prefix.

use super::python::{
    auth_from_name, callee, collect_imports, docstring, list_strings, Arguments, Constants,
    RequestDialect, RequestUsage,
};
use super::{Detection, FrameworkAdapter};
use crate::facts::{FactSink, ImportFact, MountFact, RouteFact, RouterFact, SymbolId, SymbolRef};
use crate::index::{walk, ParsedFile};
use crate::project::{Language, ProjectContext};
use rl_model::{
    AuthRequirement, BodySchema, HttpMethod, ParamStyle, PathSegment, PathTemplate, TypeHint,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use tree_sitter::Node;

/// How Django and DRF spell the request object's parts.
const REQUEST: RequestDialect = RequestDialect {
    query: &["request.GET", "request.query_params"],
    headers: &["request.headers"],
    meta_headers: &["request.META"],
    bodies: &[
        ("request.data", "application/json"),
        ("request.POST", "application/x-www-form-urlencoded"),
        ("request.FILES", "multipart/form-data"),
        ("request.body", "text/plain"),
    ],
};

/// Methods a class-based view can implement by name.
const VIEW_METHODS: &[(&str, HttpMethod)] = &[
    ("get", HttpMethod::Get),
    ("post", HttpMethod::Post),
    ("put", HttpMethod::Put),
    ("patch", HttpMethod::Patch),
    ("delete", HttpMethod::Delete),
    ("head", HttpMethod::Head),
    ("options", HttpMethod::Options),
];

/// What Django's and DRF's generic base classes serve when the subclass adds nothing.
const GENERIC_METHODS: &[(&str, &[HttpMethod])] = &[
    ("View", &[]),
    ("TemplateView", &[HttpMethod::Get]),
    ("RedirectView", &[HttpMethod::Get]),
    ("ListView", &[HttpMethod::Get]),
    ("DetailView", &[HttpMethod::Get]),
    ("FormView", &[HttpMethod::Get, HttpMethod::Post]),
    ("CreateView", &[HttpMethod::Get, HttpMethod::Post]),
    ("UpdateView", &[HttpMethod::Get, HttpMethod::Post]),
    ("DeleteView", &[HttpMethod::Get, HttpMethod::Post]),
    ("APIView", &[]),
    ("GenericAPIView", &[]),
    ("ListAPIView", &[HttpMethod::Get]),
    ("CreateAPIView", &[HttpMethod::Post]),
    ("ListCreateAPIView", &[HttpMethod::Get, HttpMethod::Post]),
    ("RetrieveAPIView", &[HttpMethod::Get]),
    ("UpdateAPIView", &[HttpMethod::Put, HttpMethod::Patch]),
    ("DestroyAPIView", &[HttpMethod::Delete]),
    (
        "RetrieveUpdateAPIView",
        &[HttpMethod::Get, HttpMethod::Put, HttpMethod::Patch],
    ),
    (
        "RetrieveDestroyAPIView",
        &[HttpMethod::Get, HttpMethod::Delete],
    ),
    (
        "RetrieveUpdateDestroyAPIView",
        &[
            HttpMethod::Get,
            HttpMethod::Put,
            HttpMethod::Patch,
            HttpMethod::Delete,
        ],
    ),
];

/// A ViewSet action: its name, the method that reaches it, and whether it needs a `pk`.
const VIEWSET_ACTIONS: &[(&str, HttpMethod, bool)] = &[
    ("list", HttpMethod::Get, false),
    ("create", HttpMethod::Post, false),
    ("retrieve", HttpMethod::Get, true),
    ("update", HttpMethod::Put, true),
    ("partial_update", HttpMethod::Patch, true),
    ("destroy", HttpMethod::Delete, true),
];

/// Which actions a ViewSet base class or mixin supplies.
const VIEWSET_BASES: &[(&str, &[&str])] = &[
    (
        "ModelViewSet",
        &[
            "list",
            "create",
            "retrieve",
            "update",
            "partial_update",
            "destroy",
        ],
    ),
    ("ReadOnlyModelViewSet", &["list", "retrieve"]),
    ("ListModelMixin", &["list"]),
    ("CreateModelMixin", &["create"]),
    ("RetrieveModelMixin", &["retrieve"]),
    ("UpdateModelMixin", &["update", "partial_update"]),
    ("DestroyModelMixin", &["destroy"]),
];

pub struct DjangoAdapter;

impl FrameworkAdapter for DjangoAdapter {
    fn id(&self) -> &'static str {
        "django"
    }

    fn languages(&self) -> &[Language] {
        &[Language::Python]
    }

    fn detect(&self, project: &ProjectContext) -> Detection {
        let mut detection = Detection::none();

        if project.has_manifest("manage.py") {
            detection.add(2, "`manage.py` is present");
        }
        if project.declares_dependency("django") {
            detection.add(1, "`django` is declared in a manifest");
        }

        let mut imported_in = None;
        let mut drf = false;
        for path in project.files_of(Language::Python) {
            let Ok(source) = project.read(path) else {
                continue;
            };
            if imported_in.is_none()
                && (source.contains("from django") || source.contains("import django"))
            {
                imported_in = Some(path.clone());
            }
            if !drf && source.contains("rest_framework") {
                drf = true;
            }
            if imported_in.is_some() && drf {
                break;
            }
        }
        if let Some(path) = imported_in {
            detection.add(
                3,
                format!("django is imported in {}", crate::project::display(&path)),
            );
        }
        if drf {
            detection.add(1, "Django REST Framework is used");
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
            // Only files that can hold a pattern list, a view, or the root setting.
            .filter(|path| {
                project.read(path).is_ok_and(|source| {
                    source.contains("urlpatterns")
                        || source.contains("ROOT_URLCONF")
                        || source.contains("View")
                        || source.contains("api_view")
                        || source.contains("request")
                        || source.contains("Router")
                })
            })
            .cloned()
            .collect()
    }

    fn extract(&self, file: &ParsedFile, sink: &mut FactSink) {
        let constants = Constants::collect(file);
        for import in collect_imports(file) {
            sink.import(import);
        }

        let mut extractor = Extractor {
            file,
            constants: &constants,
            app_name: constants.get("app_name").map(str::to_string),
            sink,
            synthetic: 0,
        };

        // Views first, so a `urls.py` that routes to a view in the same file finds it.
        extractor.views();

        let root = file.root();
        let mut cursor = root.walk();
        for statement in root.named_children(&mut cursor) {
            match statement.kind() {
                "expression_statement" => {
                    if let Some(inner) = statement.named_child(0) {
                        match inner.kind() {
                            "assignment" => extractor.assignment(inner),
                            "augmented_assignment" => extractor.assignment(inner),
                            "call" => extractor.call(inner),
                            _ => {}
                        }
                    }
                }
                // `if settings.DEBUG: urlpatterns += [...]`
                "if_statement" => {
                    walk(statement, &mut |node| {
                        if node.kind() == "augmented_assignment" {
                            extractor.assignment(node);
                        } else if node.kind() == "call" {
                            extractor.call(node);
                        }
                    });
                }
                _ => {}
            }
        }

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
    /// `app_name = "users"` in a URL conf, which names the group.
    app_name: Option<String>,
    sink: &'a mut FactSink,
    /// Counter for the names of inline pattern lists and include() bindings.
    synthetic: usize,
}

impl Extractor<'_> {
    fn text(&self, node: Node<'_>) -> &str {
        self.file.text(node)
    }

    fn reference(&self, name: &str) -> SymbolRef {
        SymbolRef::new(self.file.path.clone(), name)
    }

    fn symbol(&self, name: &str) -> SymbolId {
        SymbolId::new(self.file.path.clone(), name)
    }

    /// The group for pattern lists in this file: `app_name`, else the app directory.
    fn group(&self) -> Option<String> {
        self.app_name.clone().or_else(|| {
            let dir = self.file.path.parent()?.file_name()?.to_str()?;
            (!matches!(dir, "" | "src" | "config" | "core" | "project")).then(|| dir.to_string())
        })
    }

    /// `ROOT_URLCONF = "config.urls"`, `urlpatterns = [...]`, `urlpatterns += [...]`,
    /// `router = DefaultRouter()`.
    fn assignment(&mut self, node: Node<'_>) {
        let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        ) else {
            return;
        };
        if left.kind() != "identifier" {
            return;
        }
        let name = self.text(left).to_string();

        if name == "ROOT_URLCONF" {
            if let Some(module) = self.constants.string_value(self.file, right) {
                self.root_urlconf(&module, left);
            }
            return;
        }

        match right.kind() {
            "call" => {
                if let Some((_, function)) = callee(self.file, right) {
                    if function == "DefaultRouter" || function == "SimpleRouter" {
                        self.sink.router(RouterFact {
                            symbol: self.symbol(&name),
                            prefix: PathTemplate::empty(),
                            group: None,
                            is_app_root: false,
                            factory: None,
                            implicit: false,
                            span: self.file.span(left),
                        });
                    }
                }
            }
            "list" | "binary_operator" => {
                // A pattern list: `urlpatterns = [...]`, `= [...] + other`, `+= [...]`.
                let elements = pattern_elements(self.file, right);
                if elements.is_empty() {
                    return;
                }
                if node.kind() == "assignment" {
                    self.sink.router(RouterFact {
                        symbol: self.symbol(&name),
                        prefix: PathTemplate::empty(),
                        group: self.group(),
                        is_app_root: false,
                        factory: None,
                        implicit: false,
                        span: self.file.span(left),
                    });
                }
                for element in elements {
                    self.pattern(&name, element);
                }
            }
            _ => {}
        }
    }

    /// The application root: a mount from the settings file onto the root URL conf.
    fn root_urlconf(&mut self, module: &str, at: Node<'_>) {
        // Spelled with slashes: a dot would read as an attribute access to the graph.
        let local = format!("ROOT_URLCONF={}", module.replace('.', "/"));
        self.sink.router(RouterFact {
            symbol: self.symbol("ROOT_URLCONF"),
            prefix: PathTemplate::empty(),
            group: None,
            is_app_root: true,
            factory: None,
            implicit: false,
            span: self.file.span(at),
        });
        self.sink.import(ImportFact {
            module: self.file.path.clone(),
            local_name: local.clone(),
            source: module.to_string(),
            original: Some("urlpatterns".to_string()),
            level: 0,
        });
        self.sink.mount(MountFact {
            parent: self.reference("ROOT_URLCONF"),
            child: self.reference(&local),
            prefix: PathTemplate::empty(),
            group: None,
            auth: None,
            methods: Vec::new(),
            replaces_child_prefix: false,
            span: self.file.span(at),
        });
    }

    /// `router.register("users", UserViewSet, basename="user")`.
    fn call(&mut self, node: Node<'_>) {
        let Some((Some(object), method)) = callee(self.file, node) else {
            return;
        };
        if method != "register" {
            return;
        }
        let args = Arguments::of(self.file, node);
        let (Some(prefix), Some(viewset)) = (
            args.first_positional().or_else(|| args.keyword("prefix")),
            args.positional
                .get(1)
                .copied()
                .or_else(|| args.keyword("viewset")),
        ) else {
            return;
        };
        let prefix = self.django_path(prefix, false);
        // The registered prefix names the resource, which is the natural group.
        let group = prefix.segments.first().and_then(|s| match s {
            PathSegment::Literal { value } => Some(value.clone()),
            _ => None,
        });
        self.sink.mount(MountFact {
            parent: self.reference(&object),
            child: self.reference(self.text(viewset)),
            prefix,
            group,
            auth: None,
            methods: Vec::new(),
            replaces_child_prefix: false,
            span: self.file.span(node),
        });
    }

    /// One element of a pattern list: `path(...)`, `re_path(...)`, `url(...)`.
    fn pattern(&mut self, list: &str, element: Node<'_>) {
        let Some((_, function)) = callee(self.file, element) else {
            return;
        };
        let regex = match function.as_str() {
            "path" => false,
            "re_path" | "url" => true,
            _ => return,
        };
        let args = Arguments::of(self.file, element);
        let (Some(route), Some(view)) = (
            args.first_positional().or_else(|| args.keyword("route")),
            args.positional
                .get(1)
                .copied()
                .or_else(|| args.keyword("view")),
        ) else {
            return;
        };
        let prefix = self.django_path(route, regex);
        let span = self.file.span(element);

        // `include(...)` in its several spellings.
        if view.kind() == "call" && callee(self.file, view).is_some_and(|(_, f)| f == "include") {
            let include_args = Arguments::of(self.file, view);
            let Some(target) = include_args.first_positional() else {
                return;
            };
            // `include((patterns, "app"), namespace=...)` — the first element counts.
            let target = if target.kind() == "tuple" {
                match target.named_child(0) {
                    Some(first) => first,
                    None => return,
                }
            } else {
                target
            };
            let child = match target.kind() {
                "string" | "concatenated_string" => {
                    let Some(module) = self.constants.string_value(self.file, target) else {
                        return;
                    };
                    // Django's own contrib URL confs are outside the project by definition.
                    if module.starts_with("django.") {
                        return;
                    }
                    let local = format!("include({})", module.replace('.', "/"));
                    self.sink.import(ImportFact {
                        module: self.file.path.clone(),
                        local_name: local.clone(),
                        source: module,
                        original: Some("urlpatterns".to_string()),
                        level: 0,
                    });
                    local
                }
                "identifier" => self.text(target).to_string(),
                // `include(router.urls)`: the router itself is the thing mounted.
                "attribute" => {
                    let text = self.text(target);
                    text.strip_suffix(".urls").unwrap_or(text).to_string()
                }
                "list" => {
                    let name = format!("__patterns_{}__", self.synthetic);
                    self.synthetic += 1;
                    self.sink.router(RouterFact {
                        symbol: self.symbol(&name),
                        prefix: PathTemplate::empty(),
                        group: None,
                        is_app_root: false,
                        factory: None,
                        implicit: false,
                        span,
                    });
                    for inner in pattern_elements(self.file, target) {
                        self.pattern(&name, inner);
                    }
                    name
                }
                _ => return,
            };
            self.sink.mount(MountFact {
                parent: self.reference(list),
                child: self.reference(&child),
                prefix,
                group: None,
                auth: None,
                methods: Vec::new(),
                replaces_child_prefix: false,
                span,
            });
            return;
        }

        // `admin.site.urls` is Django's own, not the project's.
        let view_text = self.text(view);
        if view_text == "admin.site.urls" || view_text.ends_with(".site.urls") {
            return;
        }

        // A view: `views.list_users`, `UserView.as_view()`, or a lambda nobody can name.
        let child = match view.kind() {
            "identifier" | "attribute" => view_text.to_string(),
            "call" => match callee(self.file, view) {
                Some((Some(class), function)) if function == "as_view" => class,
                _ => {
                    // `path("x/", some_factory())` — a real rule with an unknowable view.
                    let mut fact =
                        RouteFact::new(self.reference(list), HttpMethod::Get, prefix, span);
                    fact.summary = Some(format!("view is `{view_text}`"));
                    self.sink.route(fact);
                    return;
                }
            },
            _ => return,
        };
        self.sink.mount(MountFact {
            parent: self.reference(list),
            child: self.reference(&child),
            prefix,
            group: None,
            auth: None,
            methods: Vec::new(),
            replaces_child_prefix: false,
            span,
        });
    }

    /// A Django route string: no leading slash, usually a trailing one, `<int:pk>`
    /// converters — or a regex for `re_path`.
    fn django_path(&self, node: Node<'_>, regex: bool) -> PathTemplate {
        let Some(text) = self.constants.string_value(self.file, node) else {
            return PathTemplate::from_segments(vec![PathSegment::unresolved(self.text(node))]);
        };
        if regex {
            regex_path(&text)
        } else {
            let mut template = PathTemplate::parse(&text, ParamStyle::Angle);
            template.trailing_slash = text.ends_with('/');
            template
        }
    }

    /// Every view in the file becomes an implicit router with one empty-path route per
    /// method it serves.
    fn views(&mut self) {
        let root = self.file.root();
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            let (definition, decorated) = match child.kind() {
                "function_definition" | "class_definition" => (child, None),
                "decorated_definition" => match child.child_by_field_name("definition") {
                    Some(d) => (d, Some(child)),
                    None => continue,
                },
                _ => continue,
            };
            match definition.kind() {
                "function_definition" => self.function_view(definition, decorated),
                "class_definition" => self.class_view(definition, decorated),
                _ => {}
            }
        }
    }

    /// A function whose first parameter is `request`, or that wears a view decorator.
    fn function_view(&mut self, definition: Node<'_>, decorated: Option<Node<'_>>) {
        let Some(name) = definition.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name).to_string();
        let decorators = decorator_names(self.file, decorated);
        let first_parameter = definition
            .child_by_field_name("parameters")
            .and_then(|p| p.named_child(0))
            .map(|p| {
                let node = if p.kind() == "identifier" {
                    p
                } else {
                    p.named_child(0).unwrap_or(p)
                };
                self.text(node).to_string()
            });
        let is_view = first_parameter.as_deref() == Some("request")
            || decorators
                .iter()
                .any(|d| d.contains("api_view") || d.contains("require_"));
        if !is_view || name.starts_with('_') {
            return;
        }

        let methods = function_methods(self.file, definition, &decorators, decorated);
        let auth = decorators.iter().find_map(|d| auth_from_decorator(d));

        self.sink.router(RouterFact {
            symbol: self.symbol(&name),
            prefix: PathTemplate::empty(),
            group: None,
            is_app_root: false,
            factory: None,
            implicit: true,
            span: self.file.span(definition),
        });
        let mut fact = RouteFact::new(
            self.reference(&name),
            methods[0].clone(),
            PathTemplate::empty(),
            self.file.span(definition),
        );
        fact.methods = methods;
        fact.auth = auth;
        fact.summary = docstring(self.file, definition);
        RequestUsage::of(self.file, definition, &[], &REQUEST).apply(&mut fact);
        self.sink.route(fact);
    }

    /// A class with view methods, a generic base, or ViewSet actions.
    fn class_view(&mut self, definition: Node<'_>, decorated: Option<Node<'_>>) {
        let (Some(name), Some(body)) = (
            definition.child_by_field_name("name"),
            definition.child_by_field_name("body"),
        ) else {
            return;
        };
        let name = self.text(name).to_string();
        let bases: Vec<String> = definition
            .child_by_field_name("superclasses")
            .map(|list| {
                (0..list.named_child_count() as u32)
                    .filter_map(|i| list.named_child(i))
                    .map(|b| {
                        let text = self.text(b);
                        text.rsplit('.').next().unwrap_or(text).to_string()
                    })
                    .collect()
            })
            .unwrap_or_default();

        let class = ClassBody::read(self.file, body);
        let is_viewset = bases.iter().any(|b| b.contains("ViewSet"))
            || VIEWSET_BASES
                .iter()
                .any(|(base, _)| bases.iter().any(|b| b == base));
        let generic: Vec<HttpMethod> = bases
            .iter()
            .filter_map(|b| GENERIC_METHODS.iter().find(|(g, _)| g == b))
            .flat_map(|(_, methods)| methods.iter().cloned())
            .collect();
        let looks_like_view = is_viewset
            || !generic.is_empty()
            || !class.methods.is_empty()
            || bases.iter().any(|b| b.ends_with("View"));
        if !looks_like_view {
            return;
        }

        let auth = class.auth.clone().or_else(|| {
            decorator_names(self.file, decorated)
                .iter()
                .find_map(|d| auth_from_decorator(d))
        });

        self.sink.router(RouterFact {
            symbol: self.symbol(&name),
            prefix: PathTemplate::empty(),
            group: None,
            is_app_root: false,
            factory: None,
            implicit: true,
            span: self.file.span(definition),
        });

        if is_viewset {
            self.viewset_routes(&name, &bases, &class, auth.as_ref(), definition);
            return;
        }

        // Methods the class defines, else what its generic base serves, else GET.
        let mut methods: Vec<(HttpMethod, Option<Node<'_>>)> = class
            .methods
            .iter()
            .filter_map(|(method_name, node)| {
                VIEW_METHODS
                    .iter()
                    .find(|(n, _)| n == method_name)
                    .map(|(_, m)| (m.clone(), Some(*node)))
            })
            .collect();
        if methods.is_empty() {
            methods = generic.iter().map(|m| (m.clone(), None)).collect();
        }
        if methods.is_empty() {
            methods.push((HttpMethod::Get, None));
        }
        if let Some(allowed) = &class.http_method_names {
            methods.retain(|(m, _)| allowed.contains(m));
        }

        for (method, function) in methods {
            let mut fact = RouteFact::new(
                self.reference(&name),
                method.clone(),
                PathTemplate::empty(),
                function.map_or_else(|| self.file.span(definition), |f| self.file.span(f)),
            );
            fact.auth = auth.clone();
            fact.summary = function
                .and_then(|f| docstring(self.file, f))
                .or_else(|| docstring(self.file, definition));
            if let Some(function) = function {
                RequestUsage::of(self.file, function, &[], &REQUEST).apply(&mut fact);
            }
            if fact.body.is_none()
                && matches!(
                    method,
                    HttpMethod::Post | HttpMethod::Put | HttpMethod::Patch
                )
            {
                fact.body = class.serializer.as_deref().map(serializer_body);
            }
            self.sink.route(fact);
        }
    }

    /// `list`/`create`/… from the bases and the body, plus `@action`s.
    fn viewset_routes(
        &mut self,
        name: &str,
        bases: &[String],
        class: &ClassBody<'_>,
        auth: Option<&AuthRequirement>,
        definition: Node<'_>,
    ) {
        let mut actions: BTreeSet<&str> = BTreeSet::new();
        for base in bases {
            if let Some((_, supplied)) = VIEWSET_BASES.iter().find(|(b, _)| b == base) {
                actions.extend(supplied.iter().copied());
            }
        }
        for (method_name, _) in &class.methods {
            if VIEWSET_ACTIONS.iter().any(|(a, _, _)| a == method_name) {
                actions.insert(method_name.as_str());
            }
        }

        let lookup = class.lookup.clone().unwrap_or_else(|| "pk".to_string());
        let detail_path = || {
            let mut template =
                PathTemplate::from_segments(vec![PathSegment::param(lookup.clone())]);
            template.trailing_slash = true;
            template
        };
        let list_path = || {
            let mut template = PathTemplate::empty();
            template.trailing_slash = true;
            template
        };

        for (action, method, detail) in VIEWSET_ACTIONS {
            if !actions.contains(action) {
                continue;
            }
            let function = class
                .methods
                .iter()
                .find(|(n, _)| n == action)
                .map(|(_, f)| *f);
            let mut fact = RouteFact::new(
                self.reference(name),
                method.clone(),
                if *detail { detail_path() } else { list_path() },
                function.map_or_else(|| self.file.span(definition), |f| self.file.span(f)),
            );
            fact.auth = auth.cloned();
            fact.summary = Some(action.to_string());
            if let Some(function) = function {
                RequestUsage::of(self.file, function, &[], &REQUEST).apply(&mut fact);
                if let Some(doc) = docstring(self.file, function) {
                    fact.summary = Some(doc);
                }
            }
            if fact.body.is_none()
                && matches!(
                    method,
                    HttpMethod::Post | HttpMethod::Put | HttpMethod::Patch
                )
            {
                fact.body = class.serializer.as_deref().map(serializer_body);
            }
            self.sink.route(fact);
        }

        // `@action(detail=True, methods=["post"], url_path="set-password")`.
        for (function_name, function, decorator) in &class.actions {
            let args = Arguments::of(self.file, *decorator);
            let detail = args
                .keyword("detail")
                .is_some_and(|n| self.text(n) == "True");
            let methods: Vec<HttpMethod> = args
                .keyword("methods")
                .map(|n| list_strings(self.file, n, self.constants))
                .unwrap_or_default()
                .iter()
                .filter_map(|m| m.parse::<HttpMethod>().ok())
                .collect();
            let methods = if methods.is_empty() {
                vec![HttpMethod::Get]
            } else {
                methods
            };
            let segment = args
                .keyword("url_path")
                .and_then(|n| self.constants.string_value(self.file, n))
                .unwrap_or_else(|| function_name.replace('_', "-"));
            let mut segments = Vec::new();
            if detail {
                segments.push(PathSegment::param(lookup.clone()));
            }
            segments.push(PathSegment::literal(segment));
            let mut path = PathTemplate::from_segments(segments);
            path.trailing_slash = true;

            let mut fact = RouteFact::new(
                self.reference(name),
                methods[0].clone(),
                path,
                self.file.span(*decorator),
            );
            fact.methods = methods;
            // `@action(..., permission_classes=[HasToken])` guards this action alone.
            fact.auth = args
                .keyword("permission_classes")
                .and_then(|list| {
                    (0..list.named_child_count() as u32)
                        .filter_map(|i| list.named_child(i))
                        .find_map(|n| auth_from_class_name(self.text(n)))
                })
                .or_else(|| auth.cloned());
            fact.summary = docstring(self.file, *function).or_else(|| Some(function_name.clone()));
            RequestUsage::of(self.file, *function, &[], &REQUEST).apply(&mut fact);
            self.sink.route(fact);
        }
    }
}

/// What a class body declares that matters here.
struct ClassBody<'a> {
    /// Methods by name, with their definitions.
    methods: Vec<(String, Node<'a>)>,
    /// `@action`-decorated methods: name, definition, the decorator call.
    actions: Vec<(String, Node<'a>, Node<'a>)>,
    auth: Option<AuthRequirement>,
    serializer: Option<String>,
    lookup: Option<String>,
    http_method_names: Option<Vec<HttpMethod>>,
}

impl<'a> ClassBody<'a> {
    fn read(file: &'a ParsedFile, body: Node<'a>) -> ClassBody<'a> {
        let mut out = ClassBody {
            methods: Vec::new(),
            actions: Vec::new(),
            auth: None,
            serializer: None,
            lookup: None,
            http_method_names: None,
        };
        let constants = Constants::default();
        let mut cursor = body.walk();

        for statement in body.named_children(&mut cursor) {
            match statement.kind() {
                "function_definition" | "decorated_definition" => {
                    let (function, decorated) = if statement.kind() == "function_definition" {
                        (statement, None)
                    } else {
                        match statement.child_by_field_name("definition") {
                            Some(d) if d.kind() == "function_definition" => (d, Some(statement)),
                            _ => continue,
                        }
                    };
                    let Some(name) = function.child_by_field_name("name") else {
                        continue;
                    };
                    let name = file.text(name).to_string();
                    let action = decorated.and_then(|d| {
                        (0..d.named_child_count() as u32)
                            .filter_map(|i| d.named_child(i))
                            .filter(|n| n.kind() == "decorator")
                            .filter_map(|n| n.named_child(0))
                            .find(|e| {
                                e.kind() == "call"
                                    && callee(file, *e).is_some_and(|(_, f)| f == "action")
                            })
                    });
                    match action {
                        Some(decorator) => out.actions.push((name, function, decorator)),
                        None => out.methods.push((name, function)),
                    }
                }
                "expression_statement" => {
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
                        "permission_classes" | "authentication_classes" => {
                            if out.auth.is_none() {
                                out.auth = (0..right.named_child_count() as u32)
                                    .filter_map(|i| right.named_child(i))
                                    .find_map(|n| auth_from_class_name(file.text(n)));
                            }
                        }
                        "serializer_class" => out.serializer = Some(file.text(right).to_string()),
                        "lookup_url_kwarg" | "lookup_field" => {
                            // `lookup_url_kwarg` wins when both are set; it is what the URL says.
                            if file.text(left) == "lookup_url_kwarg" || out.lookup.is_none() {
                                out.lookup = constants.string_value(file, right);
                            }
                        }
                        "http_method_names" => {
                            out.http_method_names = Some(
                                list_strings(file, right, &constants)
                                    .iter()
                                    .filter_map(|m| m.parse::<HttpMethod>().ok())
                                    .collect(),
                            );
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// Methods a function view serves: from `@require_*` / `@api_view`, else from the
/// `request.method == "POST"` checks in its body, else GET.
fn function_methods(
    file: &ParsedFile,
    definition: Node<'_>,
    decorators: &[String],
    decorated: Option<Node<'_>>,
) -> Vec<HttpMethod> {
    let mut methods: Vec<HttpMethod> = Vec::new();

    if let Some(decorated) = decorated {
        let mut cursor = decorated.walk();
        for decorator in decorated.children(&mut cursor) {
            if decorator.kind() != "decorator" {
                continue;
            }
            let Some(expression) = decorator.named_child(0) else {
                continue;
            };
            if expression.kind() != "call" {
                continue;
            }
            let Some((_, function)) = callee(file, expression) else {
                continue;
            };
            if function == "api_view" || function == "require_http_methods" {
                let args = Arguments::of(file, expression);
                if let Some(list) = args.first_positional() {
                    methods.extend(
                        list_strings(file, list, &Constants::default())
                            .iter()
                            .filter_map(|m| m.parse::<HttpMethod>().ok()),
                    );
                }
            }
        }
    }
    for decorator in decorators {
        match decorator.as_str() {
            "require_GET" | "require_safe" => methods.push(HttpMethod::Get),
            "require_POST" => methods.push(HttpMethod::Post),
            _ => {}
        }
    }

    if methods.is_empty() {
        // `if request.method == "POST":` — the view handles a form as well as showing it.
        methods.push(HttpMethod::Get);
        if let Some(body) = definition.child_by_field_name("body") {
            walk(body, &mut |node| {
                if node.kind() != "comparison_operator" {
                    return;
                }
                let text = file.text(node);
                if !text.contains("request.method") {
                    return;
                }
                for (name, method) in VIEW_METHODS {
                    let upper = name.to_ascii_uppercase();
                    let named = text.contains(&format!("\"{upper}\""))
                        || text.contains(&format!("'{upper}'"));
                    if named && !methods.contains(method) {
                        methods.push(method.clone());
                    }
                }
            });
        }
    }

    methods.dedup();
    methods
}

/// Decorator expressions as text, e.g. `login_required`, `api_view`, `permission_classes`.
fn decorator_names(file: &ParsedFile, decorated: Option<Node<'_>>) -> Vec<String> {
    let Some(decorated) = decorated else {
        return Vec::new();
    };
    let mut cursor = decorated.walk();
    decorated
        .children(&mut cursor)
        .filter(|n| n.kind() == "decorator")
        .filter_map(|n| n.named_child(0))
        .map(|e| file.text(e).to_string())
        .collect()
}

/// `@login_required`, `@permission_classes([IsAuthenticated])`,
/// `@method_decorator(login_required, name="dispatch")`.
fn auth_from_decorator(text: &str) -> Option<AuthRequirement> {
    if text.starts_with("login_required") || text.contains("(login_required") {
        return Some(AuthRequirement::Cookie {
            name: "sessionid".to_string(),
        });
    }
    if text.starts_with("permission_classes") || text.starts_with("authentication_classes") {
        return text
            .split(['[', ']', ',', '(', ')'])
            .map(str::trim)
            .find_map(auth_from_class_name);
    }
    if text.starts_with("csrf_exempt")
        || text.starts_with("api_view")
        || text.starts_with("require_")
    {
        return None;
    }
    auth_from_name(text.split('(').next().unwrap_or(text))
}

/// DRF permission and authentication classes by name.
fn auth_from_class_name(name: &str) -> Option<AuthRequirement> {
    let name = name.trim().rsplit('.').next().unwrap_or(name);
    match name {
        "" | "AllowAny" => None,
        "SessionAuthentication" => Some(AuthRequirement::Cookie {
            name: "sessionid".to_string(),
        }),
        "BasicAuthentication" => Some(AuthRequirement::Basic),
        "TokenAuthentication" => Some(AuthRequirement::ApiKey {
            name: "Authorization".to_string(),
            location: rl_model::ApiKeyLocation::Header,
        }),
        "JWTAuthentication" | "JSONWebTokenAuthentication" => Some(AuthRequirement::Bearer {
            format: Some("JWT".to_string()),
        }),
        "IsAuthenticated"
        | "IsAdminUser"
        | "IsAuthenticatedOrReadOnly"
        | "DjangoModelPermissions"
        | "DjangoObjectPermissions" => Some(AuthRequirement::Unknown {
            hint: name.to_string(),
        }),
        other => auth_from_name(other),
    }
}

/// `serializer_class = UserSerializer` names the body without describing it — like a
/// Pydantic model in FastAPI, the fields are runtime enrich's job.
fn serializer_body(name: &str) -> BodySchema {
    let title = name.rsplit('.').next().unwrap_or(name);
    BodySchema {
        content_type: "application/json".to_string(),
        schema: Some(serde_json::json!({ "type": "object", "title": title })),
        example: None,
        required: true,
    }
}

/// The `path(...)` calls in a list expression, looking through `+` and nesting.
fn pattern_elements<'a>(file: &ParsedFile, node: Node<'a>) -> Vec<Node<'a>> {
    let mut out = Vec::new();
    match node.kind() {
        "list" => {
            for i in 0..node.named_child_count() as u32 {
                if let Some(element) = node.named_child(i) {
                    if element.kind() == "call"
                        && callee(file, element)
                            .is_some_and(|(_, f)| matches!(f.as_str(), "path" | "re_path" | "url"))
                    {
                        out.push(element);
                    }
                }
            }
        }
        "binary_operator" => {
            for field in ["left", "right"] {
                if let Some(side) = node.child_by_field_name(field) {
                    out.extend(pattern_elements(file, side));
                }
            }
        }
        "parenthesized_expression" => {
            if let Some(inner) = node.named_child(0) {
                out.extend(pattern_elements(file, inner));
            }
        }
        _ => {}
    }
    out
}

/// A `re_path` pattern as a template: `^users/(?P<pk>\d+)/$` → `users/{pk}/`.
///
/// Named groups become parameters (typed integer when the group is `\d+`). A segment with
/// any other regex machinery is left unresolved with its text, rather than approximated.
fn regex_path(raw: &str) -> PathTemplate {
    let text = raw
        .trim()
        .trim_start_matches('^')
        .trim_end_matches('$')
        .trim_end_matches("\\Z");
    let trailing_slash = text.ends_with('/');
    let mut segments = Vec::new();

    for piece in split_regex_segments(text) {
        if piece.is_empty() {
            continue;
        }
        if let Some(inner) = piece.strip_prefix("(?P<").and_then(|s| s.strip_suffix(')')) {
            if let Some((name, pattern)) = inner.split_once('>') {
                let ty = match pattern {
                    "\\d+" | "[0-9]+" => Some(TypeHint::Integer),
                    ".+" | ".*" => Some(TypeHint::Path),
                    _ => None,
                };
                segments.push(PathSegment::Param {
                    name: name.to_string(),
                    catch_all: matches!(ty, Some(TypeHint::Path)),
                    ty,
                    optional: false,
                });
                continue;
            }
        }
        let plain = piece.replace("\\.", ".").replace("\\-", "-");
        if plain.chars().any(|c| "()[]{}*+?|\\".contains(c)) {
            segments.push(PathSegment::unresolved(piece.to_string()));
        } else {
            segments.push(PathSegment::literal(plain));
        }
    }

    let mut template = PathTemplate::from_segments(segments);
    template.trailing_slash = trailing_slash;
    template
}

/// Split on `/` outside groups, so `(?P<slug>[-\w/]+)` survives whole.
fn split_regex_segments(text: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut escaped = false;
    for c in text.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' => {
                current.push(c);
                escaped = true;
            }
            '(' | '[' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' => {
                depth -= 1;
                current.push(c);
            }
            '/' if depth <= 0 => {
                pieces.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    pieces.push(current);
    pieces
}

#[cfg(test)]
mod tests_support {
    use super::*;
    use crate::graph::RegistrationGraph;
    use crate::index::SourceIndex;

    pub fn discover(files: &[(&str, &str)]) -> (Vec<String>, Vec<String>) {
        let mut index = SourceIndex::new().unwrap();
        let mut sink = FactSink::new();
        for (path, source) in files {
            let parsed = index
                .parse(*path, Language::Python, source.to_string())
                .unwrap();
            DjangoAdapter.extract(&parsed, &mut sink);
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
}

#[cfg(test)]
mod tests {
    use super::tests_support::discover;
    use super::*;
    use crate::index::SourceIndex;

    const SETTINGS: &str = "ROOT_URLCONF = \"config.urls\"\n";

    #[test]
    fn the_root_urlconf_is_the_application_root() {
        let (routes, _) = discover(&[
            ("config/settings.py", SETTINGS),
            (
                "config/urls.py",
                "from django.urls import path\n\
                 from . import views\n\
                 urlpatterns = [path(\"health/\", views.health)]\n",
            ),
            ("config/views.py", "def health(request):\n    return {}\n"),
        ]);
        assert_eq!(routes, vec!["GET /health/"]);
    }

    #[test]
    fn include_composes_prefixes_across_apps_and_converters_are_typed() {
        let (routes, _) = discover(&[
            ("config/settings.py", SETTINGS),
            (
                "config/urls.py",
                "from django.urls import include, path\n\
                 urlpatterns = [path(\"api/\", include(\"users.urls\"))]\n",
            ),
            (
                "users/urls.py",
                "from django.urls import path\n\
                 from . import views\n\
                 app_name = \"users\"\n\
                 urlpatterns = [\n\
                 \x20   path(\"users/\", views.user_list),\n\
                 \x20   path(\"users/<int:pk>/\", views.user_detail, name=\"detail\"),\n\
                 ]\n",
            ),
            (
                "users/views.py",
                "def user_list(request):\n\
                 \x20   if request.method == \"POST\":\n\
                 \x20       return {}\n\
                 \x20   return []\n\
                 def user_detail(request, pk):\n    return {}\n",
            ),
        ]);
        assert_eq!(
            routes,
            vec![
                "GET /api/users/",
                "GET /api/users/{pk}/",
                "POST /api/users/",
            ]
        );
    }

    #[test]
    fn a_url_conf_nobody_includes_is_an_orphan_but_a_stray_view_is_not() {
        let (routes, warnings) = discover(&[
            ("config/settings.py", SETTINGS),
            (
                "config/urls.py",
                "from django.urls import path\nurlpatterns = []\n",
            ),
            (
                "old/urls.py",
                "from django.urls import path\n\
                 from . import views\n\
                 urlpatterns = [path(\"old/\", views.old)]\n",
            ),
            (
                "old/views.py",
                "def old(request): ...\n\
                 def helper(request): ...\n",
            ),
        ]);
        assert_eq!(routes, vec!["GET /old/ (orphan)"]);
        assert!(warnings
            .iter()
            .any(|w| w.contains("old/urls.py:urlpatterns")));
        assert!(
            !warnings.iter().any(|w| w.contains("helper")),
            "an unrouted view function is not reported: {warnings:?}"
        );
    }

    #[test]
    fn class_based_views_serve_their_methods_or_their_generic_base() {
        let (routes, _) = discover(&[
            ("config/settings.py", SETTINGS),
            (
                "config/urls.py",
                "from django.urls import path\n\
                 from .views import NoteView, NoteList\n\
                 urlpatterns = [\n\
                 \x20   path(\"notes/<int:pk>/\", NoteView.as_view()),\n\
                 \x20   path(\"notes/\", NoteList.as_view(), name=\"list\"),\n\
                 ]\n",
            ),
            (
                "config/views.py",
                "from django.views import View\n\
                 from django.views.generic import ListView\n\
                 class NoteView(View):\n\
                 \x20   def get(self, request, pk): ...\n\
                 \x20   def delete(self, request, pk): ...\n\
                 class NoteList(ListView):\n\
                 \x20   model = None\n",
            ),
        ]);
        assert_eq!(
            routes,
            vec!["DELETE /notes/{pk}/", "GET /notes/", "GET /notes/{pk}/"]
        );
    }

    #[test]
    fn a_drf_router_expands_a_viewset_into_its_actions() {
        let (routes, _) = discover(&[
            ("config/settings.py", SETTINGS),
            (
                "config/urls.py",
                "from django.urls import include, path\n\
                 from rest_framework.routers import DefaultRouter\n\
                 from api.views import UserViewSet, TagViewSet\n\
                 router = DefaultRouter()\n\
                 router.register(r\"users\", UserViewSet, basename=\"user\")\n\
                 router.register(\"tags\", TagViewSet)\n\
                 urlpatterns = [path(\"api/v1/\", include(router.urls))]\n",
            ),
            (
                "api/views.py",
                "from rest_framework import viewsets, mixins\n\
                 from rest_framework.decorators import action\n\
                 class UserViewSet(viewsets.ModelViewSet):\n\
                 \x20   serializer_class = UserSerializer\n\
                 \x20   permission_classes = [IsAuthenticated]\n\
                 \x20   @action(detail=True, methods=[\"post\"], url_path=\"set-password\")\n\
                 \x20   def set_password(self, request, pk=None): ...\n\
                 \x20   @action(detail=False)\n\
                 \x20   def recent(self, request): ...\n\
                 class TagViewSet(mixins.ListModelMixin, viewsets.GenericViewSet):\n\
                 \x20   lookup_field = \"slug\"\n\
                 \x20   def retrieve(self, request, slug=None): ...\n",
            ),
        ]);
        assert_eq!(
            routes,
            vec![
                "DELETE /api/v1/users/{pk}/",
                "GET /api/v1/tags/",
                "GET /api/v1/tags/{slug}/",
                "GET /api/v1/users/",
                "GET /api/v1/users/recent/",
                "GET /api/v1/users/{pk}/",
                "PATCH /api/v1/users/{pk}/",
                "POST /api/v1/users/",
                "POST /api/v1/users/{pk}/set-password/",
                "PUT /api/v1/users/{pk}/",
            ]
        );
    }

    #[test]
    fn api_view_functions_and_generics_declare_their_methods() {
        let (routes, _) = discover(&[
            ("config/settings.py", SETTINGS),
            (
                "config/urls.py",
                "from django.urls import path\n\
                 from api import views\n\
                 urlpatterns = [\n\
                 \x20   path(\"ping/\", views.ping),\n\
                 \x20   path(\"items/\", views.ItemList.as_view()),\n\
                 \x20   path(\"items/<int:pk>/\", views.ItemDetail.as_view()),\n\
                 ]\n",
            ),
            (
                "api/views.py",
                "from rest_framework.decorators import api_view\n\
                 from rest_framework import generics\n\
                 @api_view([\"GET\", \"POST\"])\n\
                 def ping(request): ...\n\
                 class ItemList(generics.ListCreateAPIView):\n\
                 \x20   serializer_class = ItemSerializer\n\
                 class ItemDetail(generics.RetrieveUpdateDestroyAPIView):\n\
                 \x20   serializer_class = ItemSerializer\n",
            ),
        ]);
        assert_eq!(
            routes,
            vec![
                "DELETE /items/{pk}/",
                "GET /items/",
                "GET /items/{pk}/",
                "GET /ping/",
                "PATCH /items/{pk}/",
                "POST /items/",
                "POST /ping/",
                "PUT /items/{pk}/",
            ]
        );
    }

    #[test]
    fn re_path_named_groups_become_parameters() {
        assert_eq!(
            regex_path(r"^users/(?P<pk>\d+)/$").render(ParamStyle::Braces),
            "/users/{pk}/"
        );
        assert_eq!(
            regex_path(r"^files/(?P<path>.+)$").render(ParamStyle::Braces),
            "/files/{path}"
        );
        let odd = regex_path(r"^v(\d)/x/$");
        assert!(!odd.is_resolved(), "an unnamed group is not guessed at");
    }

    #[test]
    fn an_inline_include_list_and_a_config_prefix() {
        let (routes, _) = discover(&[
            ("config/settings.py", SETTINGS),
            (
                "config/urls.py",
                "from django.urls import include, path\n\
                 from django.conf import settings\n\
                 from . import views\n\
                 urlpatterns = [\n\
                 \x20   path(\"legacy/\", include([path(\"ping/\", views.ping)])),\n\
                 \x20   path(settings.ADMIN_PREFIX, views.ping),\n\
                 ]\n",
            ),
            ("config/views.py", "def ping(request): ...\n"),
        ]);
        assert_eq!(routes, vec!["GET /?", "GET /legacy/ping/"]);
    }

    #[test]
    fn debug_only_patterns_and_augmented_assignment_are_read() {
        let (routes, _) = discover(&[
            ("config/settings.py", SETTINGS),
            (
                "config/urls.py",
                "from django.urls import path\n\
                 from . import views\n\
                 urlpatterns = [path(\"a/\", views.a)]\n\
                 urlpatterns += [path(\"b/\", views.b)]\n\
                 if settings.DEBUG:\n\
                 \x20   urlpatterns += [path(\"debug/\", views.a)]\n",
            ),
            (
                "config/views.py",
                "def a(request): ...\ndef b(request): ...\n",
            ),
        ]);
        assert_eq!(routes, vec!["GET /a/", "GET /b/", "GET /debug/"]);
    }

    #[test]
    fn auth_and_bodies_are_read_from_the_view() {
        let mut index = SourceIndex::new().unwrap();
        let parsed = index
            .parse(
                "api/views.py",
                Language::Python,
                "from rest_framework.views import APIView\n\
                 class Login(APIView):\n\
                 \x20   authentication_classes = [TokenAuthentication]\n\
                 \x20   def post(self, request):\n\
                 \x20       username = request.data[\"username\"]\n\
                 \x20       page = request.query_params.get(\"page\", 1)\n\
                 \x20       ua = request.META.get(\"HTTP_USER_AGENT\")\n\
                 \x20       return {}\n\
                 @login_required\n\
                 def profile(request): ...\n"
                    .to_string(),
            )
            .unwrap();
        let mut sink = FactSink::new();
        DjangoAdapter.extract(&parsed, &mut sink);

        let post = sink
            .routes
            .iter()
            .find(|r| r.router.name == "Login")
            .unwrap();
        assert!(matches!(post.auth, Some(AuthRequirement::ApiKey { .. })));
        assert_eq!(
            post.body.as_ref().unwrap().example,
            Some(serde_json::json!({"username": ""}))
        );
        assert_eq!(post.query_params[0].name, "page");
        assert_eq!(post.headers[0].name, "User-Agent");

        let profile = sink
            .routes
            .iter()
            .find(|r| r.router.name == "profile")
            .unwrap();
        assert!(matches!(profile.auth, Some(AuthRequirement::Cookie { .. })));
    }

    #[test]
    fn admin_and_contrib_urls_are_skipped_silently() {
        let (routes, warnings) = discover(&[
            ("config/settings.py", SETTINGS),
            (
                "config/urls.py",
                "from django.contrib import admin\n\
                 from django.urls import include, path\n\
                 urlpatterns = [\n\
                 \x20   path(\"admin/\", admin.site.urls),\n\
                 \x20   path(\"accounts/\", include(\"django.contrib.auth.urls\")),\n\
                 ]\n",
            ),
        ]);
        assert!(routes.is_empty());
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}
