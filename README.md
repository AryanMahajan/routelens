# RouteLens — API client that discovers endpoints from your source code

**Open a FastAPI, Flask, Django, Express or Next.js project and see every API route it
serves — then test it. A local-first, open-source API client built in Rust, with codebase-aware route
discovery instead of hand-configured collections.**

[![CI](https://github.com/AryanMahajan/routelens/actions/workflows/ci.yml/badge.svg)](https://github.com/AryanMahajan/routelens/actions/workflows/ci.yml)
![Rust](https://img.shields.io/badge/core-Rust-dea584?logo=rust&logoColor=white)
![Tauri v2](https://img.shields.io/badge/desktop-Tauri%20v2-24C8D8?logo=tauri&logoColor=white)
![Windows · macOS · Linux](https://img.shields.io/badge/platforms-Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-555)
![Status: pre-alpha](https://img.shields.io/badge/status-pre--alpha-orange)

> **Pre-alpha.** Unsigned installers for every platform are on the
> [Releases](https://github.com/AryanMahajan/routelens/releases) page, or run it from source.
> [Install](#install) · [Run it from source](#run-it-from-source) · [What works](#what-works-today) · [Docs](docs/)

---

Point RouteLens at a repository. It reads the source — it does not run it — works out which
HTTP endpoints the project exposes, and gives you a request editor for each one, linked back
to the file and line that defines it.

```
fastapi-app  ·  FastAPI  ·  http://localhost:9000

USERS
  GET     /api/v1/users/                 app/api/users.py:17
  POST    /api/v1/users/                 app/api/users.py:35    🔒 body
  GET     /api/v1/users/{user_id}        app/api/users.py:26
  DELETE  /api/v1/users/{user_id}        app/api/users.py:42    🔒
ITEMS
  GET     /v1/items                      app/api/items.py:11
  GET     /v2/items                      app/api/items.py:11    ← same router, mounted twice
ADMIN
  GET     /?/stats                       app/api/admin.py:11    ⚠ prefix comes from settings
UNGROUPED
  GET     /orphan/forgotten              app/api/orphan.py:8    ⚠ router is never mounted

14 endpoints · 2 with gaps · scanned in 86 ms
```

Every endpoint carries its method, path, query parameters, headers, request body, auth
requirement and source location. Click one, fill in the blanks, send, read the response.
No endpoint setup by hand — and where static analysis genuinely cannot know something, it
**shows the gap instead of guessing**.

## Why another API client?

Postman, Insomnia, Bruno and Hoppscotch are good at storing requests you have already
described to them. None of them read your source tree and tell you what the project actually
serves. That gap — between *"I just cloned this repo"* and *"I can call its API"* — is what
RouteLens closes.

It is deliberately not a Postman clone. It is the shortest path from *"what APIs does this
project have?"* to *"I can see it, understand it, and test it."*

|                                    | RouteLens | Postman | Insomnia | Bruno | Hoppscotch |
|------------------------------------|:---------:|:-------:|:--------:|:-----:|:----------:|
| Discovers routes from source code  | **✅**    | ✗       | ✗        | ✗     | ✗          |
| Click-through to the defining line | **✅**    | ✗       | ✗        | ✗     | ✗          |
| Works offline, no account          | ✅        | partial | partial  | ✅    | ✅         |
| Git-friendly plain-text environments | ✅      | ✗       | ✗        | ✅    | ✗          |
| Secrets kept out of committed files| ✅        | vault   | vault    | ✅    | ✗          |
| cURL / OpenAPI / raw HTTP import   | ✅        | ✅      | ✅       | ✅    | ✅         |
| Environments and `{{variables}}`   | ✅        | ✅      | ✅       | ✅    | ✅         |
| Open source                        | ✅        | ✗       | ✅       | ✅    | ✅         |

## What works today

- **Route discovery** for **FastAPI**, **Flask** (blueprints, `MethodView`, Flask-RESTful
  and RESTX), **Django + DRF** (`urlpatterns`, `include()`, class-based views, ViewSets and
  routers), **Express** (CommonJS and ESM, nested routers, `.route()` chains) and
  **Next.js** (App Router route handlers and `pages/api`), across files, following imports,
  re-exports and `include_router` / `register_blueprint` / `include()` / `app.use` prefixes.
- **Honest gaps**: a prefix read from an environment variable shows as `/?/…`, a router
  nobody mounts is flagged as an orphan, a router built by a factory is reported rather than
  dropped.
- **Ask the app** (opt-in runtime enrich, FastAPI, Flask and Django): import the
  application and take its own route table — `app.openapi()`, `url_map`, or Django's URL
  resolver — merged onto the static scan.
  Gaps get resolved, loop-registered routes appear, dead routes are labelled, and every
  source location is kept. The exact command is shown before anything runs.
- **Request editor** with tabs, path/query/header/body/auth editing, `{{variable}}`
  autocomplete, and **paste-a-cURL-into-the-URL-bar** (bash *and* Windows `cmd` quoting).
- **HTTP engine** built for predictability: redirects off by default and shown as a chain
  when on, credentials stripped on cross-origin hops, raw bytes preserved, per-request
  timeouts.
- **Environments, variables and secrets** in three tiers — committed workspace files,
  a private local secret store, disposable caches — with history recorded redacted.
  **Collections are yours**: saved requests live in your user data directory and follow
  you into every project.
- **Import** from cURL, raw HTTP and OpenAPI 3.x / Swagger 2.0.

Verified by 460+ tests, including fixture projects per framework whose snapshots record
**expected misses** as well as hits, a run against the `expressjs/express` repository
itself, and an end-to-end runtime-enrich pass over a real Flask application.

## Framework support

| Framework | Language | Status | Notes |
|---|---|---|---|
| [FastAPI](docs/discovery/frameworks.md#fastapi)  | Python | ✅ Implemented | Routers, prefixes, dependencies as auth, signatures |
| [Express](docs/discovery/frameworks.md#express)  | JS/TS  | ✅ Implemented | Module graph across `require`/`import`, mounting, chains |
| [Next.js](docs/discovery/frameworks.md#nextjs)   | TS/JS  | ✅ Implemented | App Router + legacy `pages/api`, dynamic and catch-all segments |
| [Flask](docs/discovery/frameworks.md#flask)      | Python | ✅ Implemented | Blueprints (nested, re-registered), `MethodView`, Flask-RESTful / RESTX, `add_url_rule` |
| [Django / DRF](docs/discovery/frameworks.md#django--drf) | Python | ✅ Implemented | `urlpatterns`, `include()`, `re_path`, class-based views, ViewSets, `DefaultRouter`, `@action` |

Discovery is static by default — RouteLens reads your code and never executes it. For the
Python frameworks, an opt-in [runtime enrich](docs/discovery/runtime-enrich.md) step imports
your app for an exact result when you ask for it, and always shows you the exact command
first.

Adding a framework is a self-contained job against a documented contract — see
[adding a framework](docs/discovery/adding-a-framework.md).

## Install

Download the build for your platform from the
[latest release](https://github.com/AryanMahajan/routelens/releases/latest):

| Platform | File |
|---|---|
| Windows 10/11 | `RouteLens_x.y.z_x64-setup.exe` (or the `.msi`) |
| macOS — Apple Silicon | `RouteLens_x.y.z_aarch64.dmg` |
| macOS — Intel | `RouteLens_x.y.z_x64.dmg` |
| Linux | `RouteLens_x.y.z_amd64.AppImage`, `.deb` or `.rpm` |

**The builds are not code-signed** — signing certificates cost money, and this is a free
project with no income. Each OS will warn once before the first launch:

- **Windows:** SmartScreen says "Windows protected your PC". Click **More info → Run anyway**.
- **macOS:** "cannot be opened because the developer cannot be verified". **Right-click the
  app → Open → Open**, or allow it under **System Settings → Privacy & Security**.
- **Linux:** `chmod +x RouteLens_*.AppImage`, or install the `.deb` / `.rpm` with your package
  manager.

If you would rather not run an unsigned binary, build it yourself — the release workflow is
[`.github/workflows/release.yml`](.github/workflows/release.yml), and `npm run tauri build`
produces the same installer locally.

## Run it from source

Requires a stable Rust toolchain, Node 20+, and Tauri's platform prerequisites
(WebView2 on Windows — already present on Windows 10/11; `webkit2gtk` on Linux; Xcode
command-line tools on macOS).

```bash
git clone https://github.com/AryanMahajan/routelens.git
cd routelens
npm install && npm install --prefix ui
npm run tauri dev
```

The first build compiles the Rust core and takes a few minutes; after that it is seconds.
Open any of **`tests/fixtures/{fastapi,flask,django,express,nextjs}`** for a project with
every kind of route, gap and orphan in it. The Python ones run
(`pip install -r requirements.txt`), so you can send the requests and try **Ask the app**.

## How discovery works, in three sentences

Framework adapters recognise only three things in a syntax tree — *this creates a router*,
*this registers a route*, *this mounts a router at a prefix* — plus imports and exports.
A single registration graph, shared by every framework, links those facts across files and
composes the full paths. Anything it cannot resolve statically becomes a visible
`Unresolved` segment rather than a guess.

Longer version: [how it works](docs/discovery/how-it-works.md).

## Honest limitations

- Paths built from runtime values (`settings.API_PREFIX`, `process.env.PREFIX`) are shown
  as unresolved, not guessed.
- Routes registered dynamically — in a loop, from config, by a factory — may be missed.
  Fixtures record which ones, on purpose.
- Request and response schemas are best-effort from type hints and what handlers read.
- Auth detection from middleware and dependency *names* is a heuristic and says so.

## Design principles

1. **Fast** — noticeably lighter than a full API platform. Measured, not assumed.
2. **Local-first** — no account, no cloud, everything works offline.
3. **Zero configuration where possible** — if the project already states something, read it.
4. **Codebase-aware** — always know where an endpoint actually comes from.
5. **Git-friendly** — environments and workspace settings are readable files you can
   review in a diff; collections are plain YAML too, kept per user.
6. **Extensible** — frameworks are independent adapters, never special cases in the UI.
7. **No feature bloat** — solve discovery and testing exceptionally well first.

## Documentation

Start at **[docs/](docs/)**.

- [Getting started](docs/getting-started.md) · [Concepts](docs/concepts.md) ·
  [Import](docs/import.md)
- [How discovery works](docs/discovery/how-it-works.md) ·
  [Framework support](docs/discovery/frameworks.md) ·
  [Adding a framework](docs/discovery/adding-a-framework.md)
- [Architecture](docs/architecture.md) · [Security](docs/security.md) ·
  [Workspace format](docs/workspace/format.md)

## Tech

Rust core in a Cargo workspace · Tauri v2 desktop shell · React 18 + TypeScript + Tailwind
UI · tree-sitter parsing for Python, JavaScript and TypeScript · reqwest · SQLite history.

## FAQ

**Does RouteLens run my project's code?** Not unless you ask. Discovery is static analysis
of the source. Runtime enrich is opt-in, shows you the exact command before running it, and
remembers the decision per project until you withdraw it.

**Is it a Postman alternative?** For testing the API of a codebase you have in front of you,
yes. It is not trying to replace team collaboration features, mock servers or monitoring.

**Which frameworks are supported?** FastAPI, Flask, Django (with DRF), Express and Next.js.
See [framework support](docs/discovery/frameworks.md).

**Where are my secrets stored?** Outside the committed workspace, in a private local store.
Environment files reference them by name only. See [security](docs/security.md).

## License

[MIT](LICENSE). Free to use, modify and redistribute — commercially or otherwise. RouteLens
has no paid tier, no accounts, no telemetry, and no cloud; it never will.
