"""RouteLens runtime-enrich helper.

Imports one application object and prints what it knows about its own API as JSON on
stdout. Nothing else: no server is started, no port is bound, no file is written.

    python routelens_enrich.py MODULE[:ATTRIBUTE[()]]

  MODULE           a dotted module path, imported from the current directory; for Django,
                   the settings module (what DJANGO_SETTINGS_MODULE would name)
  ATTRIBUTE        the application object inside it (default: Flask's own rules —
                   `app`, `application`, then `create_app()` / `make_app()`)
  ATTRIBUTE()      call the attribute with no arguments and use what it returns

Output, on success:

    {"framework": "fastapi" | "flask" | "django", "openapi": { ...an OpenAPI 3 document... }}

Anything the application prints during import goes to stderr, so stdout stays parseable.
Exit status is non-zero on failure, with the reason on stderr.

This file is written by RouteLens and overwritten on every run; edits will not survive.
"""

import importlib
import inspect
import json
import os
import re
import sys
import traceback


def fail(message, status=1):
    sys.stderr.write("routelens: " + message + "\n")
    sys.exit(status)


def resolve(target):
    module_name, _, attribute = target.partition(":")
    if not module_name:
        fail("no module named in target %r" % target)

    # A Django settings module is not an application object; the URL resolver is.
    if not attribute and looks_like_django_settings(module_name):
        return DjangoApp(module_name)

    try:
        module = importlib.import_module(module_name)
    except Exception:
        traceback.print_exc()
        fail("could not import %r" % module_name)

    if getattr(module, "ROOT_URLCONF", None) and not attribute:
        return DjangoApp(module_name)

    if attribute:
        call = attribute.endswith("()")
        name = attribute[:-2] if call else attribute
        try:
            obj = getattr(module, name)
        except AttributeError:
            fail("%r has no attribute %r" % (module_name, name))
        if call or (callable(obj) and not is_app(obj)):
            try:
                obj = obj()
            except Exception:
                traceback.print_exc()
                fail("calling %s:%s() failed" % (module_name, name))
        return obj

    # Flask's own lookup order, which also fits FastAPI projects well enough.
    for name in ("app", "application"):
        obj = getattr(module, name, None)
        if obj is not None and is_app(obj):
            return obj
    for name in ("create_app", "make_app"):
        factory = getattr(module, name, None)
        if callable(factory):
            try:
                return factory()
            except Exception:
                traceback.print_exc()
                fail("calling %s:%s() failed" % (module_name, name))
    fail("could not find an application object in %r; name it as MODULE:ATTRIBUTE" % module_name)


def is_app(obj):
    return hasattr(obj, "openapi") and callable(getattr(obj, "openapi")) or hasattr(obj, "url_map")


def looks_like_django_settings(module_name):
    """`config.settings`, `settings.base`, `myproject.settings.local` — without importing."""
    parts = module_name.split(".")
    return any(part.startswith("settings") for part in parts)


class DjangoApp(object):
    """Stands in for `app`: Django has no object, only a configured resolver."""

    def __init__(self, settings_module):
        self.settings_module = settings_module


def describe(app):
    if isinstance(app, DjangoApp):
        return "django", django_openapi(app.settings_module)
    if hasattr(app, "openapi") and callable(getattr(app, "openapi")):
        return "fastapi", app.openapi()
    if hasattr(app, "url_map"):
        return "flask", flask_openapi(app)
    fail("%r is neither a FastAPI, Flask nor Django application" % type(app).__name__)


# Werkzeug converter → JSON Schema.
CONVERTERS = {
    "int": {"type": "integer"},
    "float": {"type": "number"},
    "uuid": {"type": "string", "format": "uuid"},
    "path": {"type": "string", "format": "path"},
    "string": {"type": "string"},
    "default": {"type": "string"},
    "any": {"type": "string"},
}


def flask_openapi(app):
    """Build an OpenAPI 3 document from `url_map`.

    Flask keeps no schemas, so this is paths, methods, path parameters and grouping — exact
    where static analysis had to infer, and complete where it was blind (routes registered
    in a loop, or from a factory).
    """
    paths = {}
    tags = set()

    for rule in app.url_map.iter_rules():
        if rule.endpoint == "static" or rule.endpoint.endswith(".static"):
            continue

        path, parameters = convert_rule(rule)
        item = paths.setdefault(path, {})
        view = app.view_functions.get(rule.endpoint)
        doc = inspect.getdoc(view) if view is not None else None
        summary = doc.strip().splitlines()[0] if doc else None

        tag = rule.endpoint.rpartition(".")[0] or None
        if tag:
            tags.add(tag)

        methods = sorted((rule.methods or set()) - {"HEAD", "OPTIONS"})
        for method in methods:
            operation = {
                "operationId": rule.endpoint if len(methods) == 1 else "%s_%s" % (rule.endpoint, method.lower()),
                "parameters": parameters,
                "responses": {"default": {"description": ""}},
            }
            if summary:
                operation["summary"] = summary
            if tag:
                operation["tags"] = [tag]
            item[method.lower()] = operation

    return {
        "openapi": "3.0.3",
        "info": {"title": app.name, "version": ""},
        "paths": paths,
        "tags": [{"name": t} for t in sorted(tags)],
    }


RULE_PARAMETER = re.compile(
    r"<(?:(?P<converter>[a-zA-Z_][a-zA-Z0-9_]*)(?:\([^>]*\))?:)?(?P<name>[a-zA-Z_][a-zA-Z0-9_]*)>"
)


def convert_rule(rule):
    """`/users/<int:user_id>` → `/users/{user_id}` plus one parameter per converter."""
    parameters = []

    def replace(match):
        converter, name = match.group("converter"), match.group("name")
        schema = dict(CONVERTERS.get(converter or "default", {"type": "string"}))
        parameters.append({"name": name, "in": "path", "required": True, "schema": schema})
        return "{" + name + "}"

    return RULE_PARAMETER.sub(replace, rule.rule), parameters


# --- Django ---------------------------------------------------------------------------------

DJANGO_CONVERTERS = {
    "int": {"type": "integer"},
    "slug": {"type": "string", "format": "slug"},
    "uuid": {"type": "string", "format": "uuid"},
    "path": {"type": "string", "format": "path"},
    "str": {"type": "string"},
}

# `(?P<pk>[^/.]+)` in a regex pattern, DRF's routers and re_path both produce these.
REGEX_GROUP = re.compile(r"\(\?P<(?P<name>[a-zA-Z_][a-zA-Z0-9_]*)>(?P<pattern>(?:[^()]|\([^()]*\))*)\)")


def django_openapi(settings_module):
    """Walk the URL resolver Django built from `ROOT_URLCONF`.

    Methods come from the view: a ViewSet's action map, a class-based view's implemented
    handlers, an `@api_view`'s list; a plain function view says nothing and is listed as
    GET. DRF's format-suffix twins and API root are framework furniture and are dropped.
    """
    os.environ["DJANGO_SETTINGS_MODULE"] = settings_module
    try:
        import django
        from django.urls import URLResolver, get_resolver

        django.setup()
        resolver = get_resolver()
    except Exception:
        traceback.print_exc()
        fail("could not set up Django with settings %r" % settings_module)

    paths = {}
    tags = set()

    def visit(patterns, prefix, namespace):
        for entry in patterns:
            piece = str(entry.pattern)
            if isinstance(entry, URLResolver):
                inner = entry.namespace or entry.app_name or namespace
                if inner in ("admin", "djdt"):
                    continue
                visit(entry.url_patterns, prefix + piece, inner)
                continue
            if "format" in piece and ("(?P<format>" in piece or "drf_format_suffix" in piece):
                continue
            if entry.name in ("api-root",) or piece in ("", "^$") and entry.name == "api-root":
                continue
            raw = prefix + piece
            path, parameters = convert_django_path(raw)
            if path.startswith("/static/") or path.startswith("/__debug__/"):
                continue
            callback = entry.callback
            methods, doc, known = django_view_methods(callback)
            item = paths.setdefault(path, {})
            if namespace:
                tags.add(namespace)
            for method in methods:
                operation = {
                    "operationId": (entry.name or path) + ("_" + method.lower() if len(methods) > 1 else ""),
                    "parameters": parameters,
                    "responses": {"default": {"description": ""}},
                }
                if not known:
                    # A plain function view accepts whatever it is sent; nothing here says
                    # which methods it means. The static scan may know better.
                    operation["x-methods-unknown"] = True
                if doc:
                    operation["summary"] = doc
                if namespace:
                    operation["tags"] = [namespace]
                item[method.lower()] = operation

    visit(resolver.url_patterns, "", None)

    return {
        "openapi": "3.0.3",
        "info": {"title": settings_module, "version": ""},
        "paths": paths,
        "tags": [{"name": t} for t in sorted(tags)],
    }


HTTP_METHODS = {"GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE"}


def django_view_methods(callback):
    """(methods, summary, known) for whatever a URL pattern points at.

    `known` is False for a plain function view, which accepts any method unless a
    decorator says otherwise — and `require_http_methods` says so only inside a closure.
    """
    actions = getattr(callback, "actions", None)
    if actions:
        cls = getattr(callback, "cls", None)
        doc = None
        # The action's own docstring, when every method maps to the same one.
        names = set(actions.values())
        if cls is not None and len(names) == 1:
            doc = own_doc(cls, names.pop())
        if not doc and cls is not None:
            doc = own_doc(cls)
        return sorted(m.upper() for m in actions), doc, True

    cls = getattr(callback, "view_class", None) or getattr(callback, "cls", None)
    if cls is not None:
        allowed = [m for m in getattr(cls, "http_method_names", []) if m not in ("options", "head", "trace")]
        implemented = [m for m in allowed if hasattr(cls, m)]
        methods = implemented or allowed or ["get"]
        return sorted(m.upper() for m in methods), own_doc(cls), True

    doc = first_line(inspect.getdoc(callback))
    for cell in getattr(callback, "__closure__", None) or ():
        try:
            value = cell.cell_contents
        except ValueError:
            continue
        if (
            isinstance(value, (list, tuple, set, frozenset))
            and value
            and all(isinstance(v, str) and v.upper() in HTTP_METHODS for v in value)
        ):
            methods = sorted(v.upper() for v in value if v.upper() not in ("HEAD", "OPTIONS"))
            return methods or ["GET"], doc, True
    return ["GET"], doc, False


def own_doc(cls, attribute=None):
    """A docstring written in the project, not inherited from a framework base class."""
    if attribute is None:
        return first_line(cls.__dict__.get("__doc__"))
    for klass in cls.__mro__:
        if attribute in klass.__dict__:
            if klass.__module__.startswith(("rest_framework", "django")):
                return None
            return first_line(getattr(klass.__dict__[attribute], "__doc__", None))
    return None


def first_line(doc):
    if not doc:
        return None
    return doc.strip().splitlines()[0]


def convert_django_path(raw):
    """`api/users/<int:pk>/` or `api/^users/(?P<pk>[^/.]+)/$` → `/api/users/{pk}` + params."""
    parameters = []
    text = raw.replace("^", "").replace("$", "")

    def route_param(match):
        converter, name = match.group("converter"), match.group("name")
        schema = dict(DJANGO_CONVERTERS.get(converter or "str", {"type": "string"}))
        parameters.append({"name": name, "in": "path", "required": True, "schema": schema})
        return "{" + name + "}"

    def regex_param(match):
        name, pattern = match.group("name"), match.group("pattern")
        if pattern in ("\\d+", "[0-9]+"):
            schema = {"type": "integer"}
        elif pattern in (".+", ".*"):
            schema = {"type": "string", "format": "path"}
        else:
            schema = {"type": "string"}
        parameters.append({"name": name, "in": "path", "required": True, "schema": schema})
        return "{" + name + "}"

    # Regex groups first: `(?P<pk>…)` contains a `<pk>` the converter syntax would mangle.
    text = REGEX_GROUP.sub(regex_param, text)
    text = RULE_PARAMETER.sub(route_param, text)
    text = text.replace("\\.", ".").replace("\\-", "-")
    if not text.startswith("/"):
        text = "/" + text
    return text, parameters


def main():
    if len(sys.argv) != 2:
        fail("usage: routelens_enrich.py MODULE[:ATTRIBUTE[()]]", 2)

    sys.path.insert(0, os.getcwd())
    os.environ.setdefault("ROUTELENS_ENRICH", "1")

    # Anything the application prints while importing must not corrupt the JSON.
    real_stdout = sys.stdout
    sys.stdout = sys.stderr
    try:
        app = resolve(sys.argv[1])
        framework, document = describe(app)
    finally:
        sys.stdout = real_stdout

    json.dump({"framework": framework, "openapi": document}, sys.stdout)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
