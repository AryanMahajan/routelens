# Framework support

Support matrix, honest about gaps. "Planned" means designed and scheduled, not implemented.
Every implemented adapter is pinned by a fixture project under `tests/fixtures/<framework>/`
whose snapshot records what is found **and what is expected not to be**.

| Framework | Language | Phase | Status |
|---|---|---|---|
| FastAPI | Python | P3 | Implemented |
| Next.js | TS/JS | P4 | Implemented |
| Express | JS/TS | P4 | Implemented |
| Flask | Python | P5 | Implemented |
| Django / DRF | Python | P6 | Planned |

Every adapter shares the same limits, which come from static analysis itself rather than
from any one framework:

- **Nothing is executed.** A prefix read from an environment variable or a settings object
  is shown as an unresolved gap (`/?/items`), never guessed.
- **Routers built inside functions** are found, but the mount that would place them is
  not: they appear as orphans with a warning naming the call that builds them.
- **Handlers are inspected only within the file that registers them.** A controller
  defined elsewhere contributes no parameters. (Flask's class-based views are the
  exception: a `MethodView` is a router in its own right, so its methods are read where
  the class is declared.)
- **Routes registered on a function parameter** — `def register(app): app.add_url_rule(…)`
  — are kept as orphans with a warning, because where that `app` is mounted cannot be
  known.
- **Auth is a name-based heuristic.** A middleware or dependency whose name says `auth`,
  `jwt`, `token`, `protect`, `guard`… is taken as a requirement and reported as
  `Unknown` with that name, so the UI can show what was seen rather than a confident
  claim. It misfires both ways.

---

## FastAPI

**Detected by** `fastapi` in a manifest (weak), plus `from fastapi import` in source
(strong).

**Recognised**

- `app = FastAPI()` and `router = APIRouter(prefix=..., tags=[...])`
- `@app.<method>(...)` and `@router.<method>(...)` for every HTTP method
- `app.include_router(router, prefix=..., tags=..., dependencies=[...])`, nested to any
  depth, across files, through relative and absolute imports — including submodule
  imports (`from .api import admin` then `admin.router`) and re-exports
- The same router mounted at two prefixes yields both paths
- `add_api_route(...)` and `add_route(...)`
- Path parameters with converters, e.g. `{user_id:int}`, `{path:path}`
- Query parameters from defaulted handler arguments and `Query(...)`; headers from
  `Header(...)`
- Request bodies from Pydantic-looking annotations and `Body(...)`
- `Depends(...)` / `Security(...)` on security-looking names as an auth signal, on a route
  or on an `include_router`
- `summary`, `description`, docstrings, `deprecated`, `tags` as the grouping

**Gaps**

- Prefixes read from settings objects: shown unresolved
- Routers built by factory functions: found as orphans, with a warning
- Pydantic model *contents* are not reconstructed — the body is known to be JSON of that
  model, not its fields
- Custom `APIRoute` subclasses that rewrite paths

**Runtime enrich available** — `app.openapi()` supersedes every schema gap above, and the config-derived prefix becomes a resolved path with its source line kept. See [runtime enrich](runtime-enrich.md).

---

## Next.js

**Detected by** `next` in `package.json`, a `next.config.*` file, and — the strongest
signal — route files where Next's conventions put them.

The cheapest adapter, because the path *is* the file system. The syntax tree is only
consulted for which methods a file handles.

**Recognised**

- App Router: `app/**/route.{ts,js,tsx,jsx,mjs}`, also under `src/app/` and in monorepo
  sub-apps (`apps/web/app/`)
- Dynamic `[id]`, catch-all `[...slug]`, optional catch-all `[[...slug]]`
- Route groups `(group)`, parallel slots `@slot` and intercept markers `(.)`, all absent
  from the URL; private folders `_name`, which are not routes at all
- Method handlers in every export spelling: `export async function GET`, `export const
  POST = …`, `export { handler as DELETE }`, `export const { PUT } = …`, re-exports
- What a handler reads: `searchParams.get("q")` → query, `request.headers.get(…)` →
  header, `request.json()` / `formData()` / `text()` → body type
- Pages Router: `pages/api/**` with a default export, `index` files, dynamic file names,
  underscore-prefixed files skipped
- The Pages handler's accepted methods from `req.method === "POST"`, `!==` guards,
  `switch (req.method)` and `["GET","POST"].includes(req.method)`; a handler that never
  checks is listed under GET/POST/PUT/PATCH/DELETE with a summary saying why
- `req.query` / `req.body` / `req.headers` usage in Pages handlers, with dynamic-segment
  names correctly excluded from the query list
- `export default withAuth(handler)`: the wrapped handler is followed, and the wrapper's
  name is an auth hint
- Port from `next dev -p N` in `package.json`, else 3000

**Gaps**

- `basePath` and `rewrites` in `next.config.*` are not applied
- Body *shapes* are not reconstructed, even from zod
- Edge `middleware.ts` is not modelled — it is not an endpoint, and its `matcher` is not
  used to annotate routes

---

## Express

**Detected by** `express` in `package.json` (weak), plus an `express` require or import
(strong).

Structurally the hardest adapter — the module graph is load-bearing, and it is the same
graph FastAPI uses. Express adds CommonJS/ESM export resolution on top.

**Recognised**

- `const app = express()`; `express.Router()`, `Router()`, `new Router()`, with or without
  options
- `app.<method>(path, ...handlers)` and `router.<method>(...)`; `.all` listed under
  GET/POST/PUT/PATCH/DELETE with a summary saying why
- `app.route(path).get(a).post(b)` chains, and `app.get(...).post(...)` chains
- Arrays of paths: `app.get(["/a", "/b"], h)`
- `app.use(prefix, router)`, `app.use(router)`, `app.use(prefix, require("./routes"))`,
  arrays of routers, the same router mounted twice, nesting to any depth
- Module linking in both systems: `require`, destructured `require`,
  `require("./m").name`, `import x from`, `import { a as b }`, `import * as`; exports via
  `module.exports = x`, `module.exports = { a, b: c }`, `exports.a = …`, `export default`,
  `export const`, `export { a as b }`, and `export … from` re-exports; `@/` and `~/`
  aliases tried from `src/` and the root
- Path constants folded from literals, template strings and `+`
- Path parameters `:id`, optional `:id?`, wildcard `*`
- What a handler reads off `req`: `req.query.x` and `const { x } = req.query` → query,
  `req.body.x` → body fields (offered as an example body), `req.headers["x"]` and
  `req.get("X")` → headers, with `Authorization` promoted to an auth requirement
- Auth middleware in a handler chain, ahead of a mounted router (`app.use("/admin",
  requireAuth, adminRouter)` guards everything under it), or applied router-wide with
  `router.use(requireAuth)`
- Package middleware (`cors()`, `helmet`, `express.json()`) recognised as not-a-router
  and ignored quietly
- Port from `PORT=` in `.env*`, `-p`/`--port`/`PORT=` in run scripts, else 3000

**Gaps**

- A router built by a call (`app.use("/api", createRouter())`) cannot be followed:
  the mount is reported with a warning, and the routes inside the factory appear as
  orphans
- `this.router` in class-based controllers: reported, not guessed
- Regex paths and `process.env` prefixes: shown unresolved
- Handlers in other files (`users.list` from a controllers module) contribute no
  parameters
- Request schemas from zod, joi or TypeScript types are not read

---

## Flask

**Detected by** `flask` in the manifest (+1), plus a `from flask import` in source (+3).

Reuses the Python resolver built for FastAPI; a `Blueprint` maps onto the same router
concept. Pinned by `tests/fixtures/flask/` — a runnable application with an app factory,
nested blueprints, a blueprint registered twice, a config-derived prefix, a `MethodView`
in another file, an unregistered blueprint, and routes registered in a loop.

**Recognised**

- `app = Flask(__name__)`, at module level or inside a factory (`def create_app()`)
- `Blueprint(name, __name__, url_prefix=...)`; the name becomes the group
- `@app.route(path, methods=[...])` and `@bp.route(...)`, defaulting to `GET`
- `@app.get/post/put/patch/delete(...)` shortcuts (Flask ≥ 2.0)
- `app.register_blueprint(bp, url_prefix=...)`, including nested `bp.register_blueprint(child)`
- `app.add_url_rule(rule, endpoint, view_func, methods=[...])`
- Class-based views: `MethodView` subclasses registered with `view_func=X.as_view(...)`
  (through a local alias too), one route per `get`/`post`/… method; `methods = [...]`
  narrows them; `decorators = [...]` guards them; `methods=` on the rule narrows that rule
- Flask-RESTful: `Api(app, prefix=...)`, `api.init_app(app)`, `api.add_resource(Cls, *rules)`
- Flask-RESTX: `Namespace(name, path=...)`, `api.add_namespace(ns, path=...)`,
  `@ns.route(...)` on a `Resource` class
- Werkzeug converters — `<int:user_id>`, `<path:subpath>` (catch-all), `<uuid:id>`
- Query parameters from `request.args.get("q")` / `request.args["q"]` (the latter
  required); headers from `request.headers.get(...)`; a JSON body from
  `request.get_json()` / `request.json` with the keys the handler reads as the example;
  a form body from `request.form`; multipart from `request.files`
- Auth from decorators: `@login_required` (session cookie), `@jwt_required()` (bearer),
  `@basic_auth.login_required` (basic), `@token_required` and anything else whose name
  says auth (reported as `Unknown` with the name)
- The handler docstring as the summary

**Prefix semantics, which differ from FastAPI:** `register_blueprint(bp, url_prefix="/x")`
*replaces* the blueprint's own `url_prefix`; only when none is given does the blueprint's
apply. Nested blueprints compose. This is what Werkzeug does, and it was found by comparing
the scan with the fixture's real `url_map` — the adapter says so to the graph through
`MountFact.replaces_child_prefix`.

**Gaps**

- Schemas: Flask has no schema layer, and none is inferred from Marshmallow or Pydantic.
  Runtime enrich does not add any either — `url_map` knows paths and methods only.
- Routes registered in a loop or from data are listed as an unresolved orphan (`GET /?`)
  with a warning, not expanded. Runtime enrich lists them exactly.
- `@bp.before_request` guards are not read as auth.
- Flask-RESTX `@ns.expect(model)` / `@ns.doc(...)` are not read.
- A `view_func` imported from a package outside the project is reported as an
  undeclared mount and yields nothing.

**Runtime enrich available** — walks `app.url_map` for a complete and exact route list,
including the loop-registered and config-prefixed ones above. See
[runtime enrich](runtime-enrich.md).

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
