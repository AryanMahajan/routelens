# Framework support

Support matrix, honest about gaps. "Planned" means designed and scheduled, not implemented.

| Framework | Language | Phase | Status |
|---|---|---|---|
| FastAPI | Python | P3 | Planned |
| Next.js | TS/JS | P4 | Planned |
| Express | JS/TS | P4 | Planned |
| Flask | Python | P5 | Planned |
| Django / DRF | Python | P6 | Planned |

---

## FastAPI

**Detected by** `fastapi` in `pyproject.toml` / `requirements.txt`, plus `from fastapi import`.

**Recognised**

- `@app.get/post/put/patch/delete/head/options(...)`
- `@router.<method>(...)` on `APIRouter` instances
- `APIRouter(prefix=..., tags=...)`
- `app.include_router(router, prefix=..., tags=...)`, including nested inclusion
- `app.add_api_route(...)` and `router.add_api_route(...)`
- Path parameters with type hints and converters, e.g. `{user_id:int}`
- Query parameters via `Query(...)` and defaulted handler arguments
- Request bodies from Pydantic models and `Body(...)`
- `response_model` for response schema
- `Depends(...)` on security utilities as an authentication signal
- `tags` used as the tree grouping

**Gaps**

- Routers built by factory functions or in loops
- Prefixes read from settings objects, which are shown unresolved
- Complex Pydantic generics and discriminated unions reduce to partial schemas
- Custom `APIRoute` subclasses that rewrite paths

**Runtime enrich available** — `app.openapi()` supersedes every schema gap above.

---

## Next.js

**Detected by** `next` in `package.json`, or a `next.config.*` file.

The cheapest adapter, because paths come from the filesystem rather than from code.

**Recognised**

- App Router: `app/api/**/route.ts` (also `.js`, `.tsx`)
- Dynamic segments `[id]`, catch-all `[...slug]`, optional catch-all `[[...slug]]`
- Route groups `(group)` — present in the tree, absent from the URL
- Exported method handlers: `export async function GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS`
- Legacy Pages Router: `pages/api/**` with a default-exported handler
- `basePath` from `next.config.*` where statically resolvable
- Route handler response types as a weak response-schema signal

**Gaps**

- Legacy handlers branch on `req.method` at runtime. The method set is inferred where the
  branching is a simple `switch` or `if` chain, and left as "any method" otherwise
- Middleware rewrites, and `rewrites`/`redirects` in `next.config`
- Request body schemas, unless zod or an annotated type is used

---

## Express

**Detected by** `express` in `package.json`, plus an `express` require or import.

Structurally the hardest adapter — the module graph is load-bearing.

**Recognised**

- `app.<method>(path, ...handlers)` and `router.<method>(...)`
- `express.Router()` instances assigned to variables and exported
- `app.use(path, router)` mounting, including inline require-mounts
- `app.route(path).get(...).post(...)` chains
- Nested router mounting to arbitrary depth
- CommonJS and ESM module linking
- Path parameters `:id`, optional `:id?`, and wildcards
- Auth middleware in a handler chain as an authentication signal

**Gaps**

- Routers assembled dynamically, or re-exported through barrel files with renaming
- Paths built by string concatenation with runtime values, which are shown unresolved
- Regex route paths — recorded, but not turned into a fillable template
- Request schemas, unless zod, joi, or TypeScript types are used recognisably

---

## Flask

**Detected by** `flask` in the manifest, plus a Flask import.

Reuses the Python resolver built for FastAPI; Blueprints map onto the same router concept.

**Recognised**

- `@app.route(path, methods=[...])`, defaulting to `GET` when `methods` is omitted
- `@bp.route(...)` on `Blueprint` instances
- `Blueprint(name, __name__, url_prefix=...)`
- `app.register_blueprint(bp, url_prefix=...)`, including nested registration
- `app.add_url_rule(...)`
- Werkzeug converters such as `<int:user_id>` and `<path:subpath>`

**Gaps**

- Schemas — Flask has no built-in schema layer, so only recognisable Marshmallow or
  Flask-Pydantic usage yields anything
- Application-factory patterns where the app is built inside a function with conditional
  blueprint registration
- `MethodView` and pluggable views are only partially recognised

**Runtime enrich available** — walks `app.url_map` for a complete route list.

---

## Django / DRF

**Detected by** `manage.py`, `django` in the manifest, or a settings module with
`ROOT_URLCONF`.

**Recognised**

- `urlpatterns` lists in URL configuration modules
- `path()` and `re_path()` with converters such as `<int:pk>` and `<slug:name>`
- `include()` composition across URL confs, following `ROOT_URLCONF`
- DRF `@api_view([...])`
- DRF viewsets registered on a `DefaultRouter` or `SimpleRouter`, expanded into their
  generated route set: list, create, retrieve, update, partial update, destroy
- `@action` decorators on viewsets
- DRF serializers as request and response schema
- `permission_classes` as an authentication signal

**Gaps**

- `urlpatterns` built conditionally or extended at import time
- Custom router classes with non-standard route generation
- Non-DRF class-based views yield methods but rarely schemas
