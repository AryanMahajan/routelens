# RouteLens Documentation

> **These documents describe a design that is being built, not a shipped product.**
> Where a document describes something not yet implemented, it says so. Anything in `docs/`
> is a promise the code is expected to keep — if code and docs disagree, that is a bug in one
> of them.

## Project status

Implementation is phased so each phase leaves a usable application.

| Phase | Contents | Status |
|---|---|---|
| P0 | Crate skeleton, unified model, storage tiers, variable resolver, docs | In progress |
| P1 | HTTP engine, request editor, response viewer, history | Not started |
| P2 | cURL / raw HTTP / OpenAPI importers | Not started |
| P3 | Discovery core + FastAPI adapter + explorer | Not started |
| P4 | Next.js adapter, then Express adapter | Not started |
| P5 | Runtime enrich, Flask adapter | Not started |
| P6 | Collections & environments UI, Django + DRF | Not started |

Live phase tracking, including what slipped and why, lives in `docs/internal/roadmap.md`
(not committed).

## Reading order

**If you want to use RouteLens**

1. [Getting started](getting-started.md) — install, open a project, send a request
2. [Concepts](concepts.md) — the two core types and why there are two
3. [Import](import.md) — cURL, OpenAPI, raw HTTP
4. [Workspace format](workspace/format.md) — what lands on disk
5. [Environments](workspace/environments.md) and [secrets](workspace/secrets.md)

**If you want to understand or contribute to it**

1. [Architecture](architecture.md) — crates, boundaries, data flow
2. [How discovery works](discovery/how-it-works.md) — the registration graph
3. [Framework support](discovery/frameworks.md) — what each adapter handles and misses
4. [Adding a framework](discovery/adding-a-framework.md) — the adapter contract
5. [Runtime enrich](discovery/runtime-enrich.md) — the opt-in high-fidelity path
6. [Security](security.md) — the trust boundaries

## Documentation tiers

- **`README.md`** (repo root) — what RouteLens is, for someone who has never seen it.
- **`docs/`** — committed, public, kept in sync with the code.
- **`docs/internal/`** — gitignored working notes: decision records, parsing spikes, live
  roadmap. Messy on purpose; nothing there is a promise.
