"""RouteLens runtime-enrich helper.

Imports one application object and prints what it knows about its own API as JSON on
stdout. Nothing else: no server is started, no port is bound, no file is written.

    python routelens_enrich.py MODULE[:ATTRIBUTE[()]]

  MODULE           a dotted module path, imported from the current directory
  ATTRIBUTE        the application object inside it (default: Flask's own rules —
                   `app`, `application`, then `create_app()` / `make_app()`)
  ATTRIBUTE()      call the attribute with no arguments and use what it returns

Output, on success:

    {"framework": "fastapi" | "flask", "openapi": { ...an OpenAPI 3 document... }}

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

    try:
        module = importlib.import_module(module_name)
    except Exception:
        traceback.print_exc()
        fail("could not import %r" % module_name)

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


def describe(app):
    if hasattr(app, "openapi") and callable(getattr(app, "openapi")):
        return "fastapi", app.openapi()
    if hasattr(app, "url_map"):
        return "flask", flask_openapi(app)
    fail("%r is neither a FastAPI nor a Flask application" % type(app).__name__)


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
