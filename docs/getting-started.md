# Getting started

> **Pre-alpha.** There are no release builds yet; run it from source. Steps marked
> *(planned)* do not work yet.

## Run from source

```bash
git clone https://github.com/AryanMahajan/routelens.git
cd routelens
npm install --prefix ui
npm run tauri dev                  # development build, hot-reloading UI
npm run tauri build                # installer / bundle *(untested so far)*
```

The first `dev` compiles the Rust side, which takes a few minutes; afterwards it is seconds.
The sample projects under `tests/fixtures/` — `fastapi`, `nextjs`, `express` — are the
quickest things to open: each contains routes that should be found, deliberate gaps that
should be *shown* rather than guessed, and an unmounted router that should be flagged.

Requires a recent stable Rust toolchain and Node 20+. Tauri also needs platform
prerequisites — WebView2 on Windows, `webkit2gtk` on Linux, Xcode command line tools on macOS.

## Open a project

Choose **Open Project** and select a repository root. RouteLens will:

1. Detect the language and framework from manifests and imports.
2. Scan the source for route registrations.
3. Resolve router prefixes into full paths.
4. Infer candidate base URLs (from run scripts, `.env`, Dockerfile, compose files).
5. Show you the endpoint tree.

A first scan of a mid-sized project should complete in well under a second; rescans are
incremental and near-instant.

RouteLens only reads files inside the directory you selected, and it does not execute your
project's code unless you explicitly ask for [runtime enrich](discovery/runtime-enrich.md).

## Read an endpoint

Selecting an endpoint shows everything discovery could establish:

- Method and full path, with path parameters called out
- Query parameters, headers, and request body schema where detectable
- Authentication requirement where detectable
- The **file and line** it was defined on — click to open it in your editor
- Which framework and which discovery source it came from

Where discovery could not establish something, the UI says so. An unresolvable path segment
appears as `?` with the source expression, rather than a guess.

## Save the whole API as a collection

**Save all** in the API panel writes every resolved endpoint into one collection, one
folder per group (router, blueprint, tag or app), and switches to **Collections** to show
it. Endpoints with an unresolved path are skipped and counted rather than written as
`/?/stats`. Run it again after a rescan and it updates the discovered requests in place —
matched by the endpoint they came from — while leaving your own edits, folders and
hand-added requests alone.

## Ask the application (FastAPI, Flask and Django)

Static analysis stops where the code needs running: a prefix from `settings.API_PREFIX`,
routes registered in a loop, Pydantic schemas. For FastAPI, Flask and Django projects the
endpoint list offers **Ask the app**. It shows the exact command it will run — interpreter, helper
script, `module:app` target — and where each part came from, and waits for you to press
**Run**. Nothing is executed before that.

The helper imports your application, asks it for its own route table (`app.openapi()` or
`url_map`), and exits. The result is merged onto the static scan: exact paths and schemas,
source locations kept, and every difference labelled — `RT` for a route only the
application knows about, `✓` for a gap the application closed, `∅` for something declared
in source that the application does not serve.

The target is remembered in `.routelens/workspace.yaml`; **Forget** in the dialog withdraws
it. Rescanning returns to the static result. Details in
[runtime enrich](discovery/runtime-enrich.md).

## Send a request

1. Pick a base URL — RouteLens suggests candidates it found; you can override it.
2. Fill in path parameters. Adjust query, headers, and body as needed.
3. Set auth, or reference an environment variable such as `{{token}}`.
4. **Send.**

The response pane shows status, timing, response headers, and the body — pretty-printed,
raw, or saved to a file for large payloads.

## Save it

**Save** (Ctrl+S) writes the request into a collection — the box next to the button names
which; type a new name to create one. A request opened from a collection saves back to it.
Collections are yours, not the project's: they live in your user data directory
(`%LOCALAPPDATA%\routelens\collections` on Windows, `~/.local/share/routelens/collections`
on Linux) and the same list appears in every project you open. Environments, secrets and
history stay with the project.

The **Collections** panel is where they are managed: drag a request to reorder it, drop it
on a folder or another collection to move it, and use the `…` menu on a request to rename,
duplicate, delete or file it under a folder (`Users/Admin` nests). The `…` on a collection
renames or deletes it; **+ Collection** makes an empty one.

## Paste a cURL command

Choose **Import → cURL** and paste:

```bash
curl 'https://api.example.com/users?page=2' \
  -H 'Authorization: Bearer token' \
  -H 'Content-Type: application/json' \
  --data-raw '{"name":"Aryan"}'
```

RouteLens splits this into method, URL, query parameters, headers, recognised auth, and body
automatically — you never sort the pieces by hand. See [import](import.md).

## Work without a project

Choose **New Workspace** to use RouteLens as a standalone API client — collections,
environments, variables, and history, with no project attached.
