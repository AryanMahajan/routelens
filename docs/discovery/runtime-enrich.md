# Runtime enrich

Static analysis reads your code. Runtime enrich *asks your application*. It is more accurate
and more invasive, so it is always opt-in.

## Why it exists

FastAPI already knows its own API perfectly. `app.openapi()` returns a complete OpenAPI
document with every path, parameter, schema, and security scheme, generated from the same
objects that serve the requests. No static analysis will ever match that.

Flask exposes `app.url_map`, which gives a complete and exact route list, though no schemas.

When fidelity matters, going to the source of truth beats inferring it.

## What it costs

Runtime enrich **imports your project's code and executes it**. Module-level code runs. That
can mean database connections, config validation, network calls, or any other import-time
side effect your application has.

This is a real trust boundary, so:

- It is **never automatic**. Nothing runs until you ask for it.
- Before the first run for a project, RouteLens shows the **exact command** it will execute
  and the interpreter it will use, and waits for confirmation.
- The decision is stored per project and can be revoked.
- If the helper fails, the failure is reported with its stderr, and static results are left
  intact.

See [security](../security.md) for the full trust model.

## How it works

1. **Locate the app object.** RouteLens needs a `module:attribute` target — the same thing
   `uvicorn` takes. It infers this from `uvicorn` or `gunicorn` arguments in scripts,
   Procfiles, or compose files. If inference is ambiguous it asks once, then stores the answer
   in `.routelens/`.

2. **Choose the interpreter.** The project's own virtualenv, detected from `.venv`, `venv`,
   Poetry, or an active environment. The helper has to run where the project's dependencies
   are installed.

3. **Run a helper script.** A small, self-contained script imports the target, extracts the
   API description, and writes JSON to stdout. It introspects only: it starts no server and
   binds no port.

   - **FastAPI** — calls `app.openapi()`
   - **Flask** — walks `app.url_map`, plus Marshmallow or Pydantic schemas where present

4. **Import the result.** The JSON goes through the same [OpenAPI importer](../import.md)
   used for ordinary spec imports. This reuse is deliberate: the importer is built in an
   earlier phase precisely so this step is nearly free.

5. **Merge with static results.**

## The merge

Runtime results do not replace static results. The two are unioned, keyed on
`(method, normalized_path)`:

| Field | Winner | Why |
|---|---|---|
| Request and response schema | **Runtime** | Generated from the real models |
| Parameters | **Runtime** | Exact, including ones inference missed |
| Auth requirement | **Runtime** | Real security schemes |
| Source file and line | **Static** | Runtime does not know where code lives |
| Grouping and tags | Runtime, falling back to static | |

Endpoints found only by static analysis are kept and flagged. Endpoints found only at runtime
are kept and flagged as having no source location — those are usually exactly the
dynamically-registered routes static analysis is blind to, which makes the difference between
the two lists genuinely informative rather than noise.

The result is the best of both: exact schemas *and* click-through to source.

## When you do not need it

- The project is Next.js or Express, where there is no runtime spec to fetch and static
  analysis is the whole story.
- You only need paths and methods, which static analysis gets right in the common case.
- You cannot or would rather not install the project's dependencies.

RouteLens is fully usable without ever enabling it.
