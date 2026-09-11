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

## Send a request

1. Pick a base URL — RouteLens suggests candidates it found; you can override it.
2. Fill in path parameters. Adjust query, headers, and body as needed.
3. Set auth, or reference an environment variable such as `{{token}}`.
4. **Send.**

The response pane shows status, timing, response headers, and the body — pretty-printed,
raw, or saved to a file for large payloads.

## Save it

Any request you have executed can be saved into a collection, which is written as readable
YAML under `.routelens/` in the project. That file is designed to be committed, so the next
person who clones the repo starts with your requests already there.

Secret values never go into those files. See [secrets](workspace/secrets.md).

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
