# RouteLens

**Discover and interact with APIs directly from your codebase.**

> **Status: pre-alpha.** Design is settled and documented; implementation is in progress.
> Nothing below is installable yet. See [the roadmap](docs/README.md#project-status) for what
> actually works today.

---

Point RouteLens at a project. It reads the source, works out what HTTP API the project
exposes, and lets you call those endpoints immediately.

```
routelens .
```

```
myproject  ·  FastAPI  ·  http://localhost:8000

API
├── Auth
│   ├── POST    /api/v1/login
│   └── POST    /api/v1/refresh
│
├── Users
│   ├── GET     /api/v1/users
│   ├── GET     /api/v1/users/{user_id}
│   ├── POST    /api/v1/users
│   └── DELETE  /api/v1/users/{user_id}
│
└── Documents
    ├── GET     /api/v1/documents
    └── POST    /api/v1/documents

12 endpoints · 1 unresolved · scanned in 240ms
```

Every endpoint carries its method, path and query parameters, request schema, auth
requirement, and the file and line it was defined on. Select one, fill in the values, send it,
read the response. No manual endpoint setup.

## Why

Existing API clients are excellent at *storing* requests you have already described to them.
None of them read your source tree and tell you what the project actually serves. That gap —
between "I just cloned this repo" and "I can call its API" — is what RouteLens closes.

The goal is not to build another Postman. The goal is the shortest path from
*"what APIs does this project have?"* to *"I can see it, understand it, and test it."*

## What it does

**Project mode** — open a repository and get an explorer of its real endpoints, each linked
back to its definition in source.

**Workspace mode** — a general-purpose API client: collections, saved requests, environments,
variables, history. Works with no project at all.

**Import mode** — paste a cURL command, an OpenAPI document, or a raw HTTP request and get a
structured, editable, executable request. You never have to decide by hand whether something
belongs in headers, query, auth, or body.

All four sources — source code, cURL, OpenAPI, manual — resolve into one internal model, so
the rest of the app treats them identically.

## Framework support

Discovery is static by default: RouteLens reads your code, it does not run it. An optional
[runtime enrich](docs/discovery/runtime-enrich.md) step can import your app for a
higher-fidelity result when you ask for it.

| Framework | Language | Status | Notes |
|---|---|---|---|
| FastAPI  | Python | Implemented | Routers, prefixes, signatures; runtime enrich planned |
| Next.js  | TS/JS  | Implemented | App Router + legacy `pages/api` |
| Express  | JS/TS  | Implemented | Router mounting across CommonJS and ESM modules |
| Flask    | Python | Planned — P5 | Blueprints; runtime enrich available |
| Django   | Python | Planned — P6 | `urlpatterns`, `include()`, DRF routers |

Adding a framework is meant to be a small, self-contained job — see
[adding a framework](docs/discovery/adding-a-framework.md).

## Design principles

1. **Fast** — noticeably lighter than a full API platform. Measured, not assumed.
2. **Local-first** — no account, no mandatory cloud, everything works offline.
3. **Zero configuration where possible** — if the project already states something, discover
   it rather than asking.
4. **Codebase-aware** — always know where an endpoint actually comes from.
5. **Git-friendly** — workspace definitions are readable files you can review in a diff.
6. **Extensible** — frameworks are independent adapters, never special cases scattered
   through the UI.
7. **No feature bloat** — solve discovery and testing exceptionally well first.

## Honest limitations

Static analysis reads code without executing it, which has real limits:

- Paths built from values RouteLens cannot resolve (`PREFIX = settings.API_PREFIX`) are shown
  as **unresolved** rather than guessed at. A visible gap beats a silently wrong route.
- Routes registered dynamically — in a loop, from config, by a factory — may be missed.
- Request and response schemas are best-effort from type hints and annotations.

Runtime enrich closes most of this by asking your app directly, at the cost of executing your
project's code. It is always opt-in and always tells you exactly what it will run.

## Documentation

Start at **[docs/](docs/)**.

- [Getting started](docs/getting-started.md) — install, first project, first request
- [Concepts](docs/concepts.md) — the unified model
- [How discovery works](docs/discovery/how-it-works.md) — the interesting part
- [Architecture](docs/architecture.md) — crate map and data flow
- [Security](docs/security.md) — what RouteLens reads, runs, and stores

## Tech

Rust core · Tauri v2 desktop shell · React + TypeScript UI · tree-sitter parsing ·
SQLite for history and index caches.

## License

Not yet chosen.
