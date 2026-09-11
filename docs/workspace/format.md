# Workspace format

Everything RouteLens stores lands in a `.routelens/` directory. The layout splits into three
tiers by how the data should be treated: shared, private, and disposable.

```
.routelens/
├── workspace.yaml            # committed — workspace identity and settings
├── collections/
│   └── users.yaml            # committed — saved requests
├── environments/
│   ├── local.yaml            # committed — variable names, non-secret values
│   └── staging.yaml          # committed
├── .gitignore                # committed — ignores local/
└── local/                    # never committed
    ├── secrets.json          # secret values (or an OS keychain reference)
    ├── history.sqlite        # request history
    └── index.sqlite          # source index cache — disposable, rebuildable
```

RouteLens writes `.routelens/.gitignore` automatically on workspace creation, so the private
tier is excluded from the moment it exists rather than after someone notices.

## Why three tiers

Two tiers is the obvious design and it is wrong. "Committed" and "not committed" leaves no
place for the distinction between *a secret you must protect* and *a cache you can delete* —
and conflating them means either caches get backed up or secrets get treated as disposable.

| Tier | Contents | Committed | Safe to delete |
|---|---|---|---|
| Shared | Collections, environments, settings | Yes | No — real work |
| Private | Secret values | No | No — real credentials |
| Disposable | History, source index | No | Yes — rebuilds itself |

## `workspace.yaml`

```yaml
version: 1
name: myproject
kind: project            # project | standalone
project:
  root: .
  frameworks: [fastapi]
  app_target: app.main:app      # for runtime enrich, if confirmed
default_environment: local
```

## Collections

One YAML file per collection, designed to read well in a diff. Requests appear in the order
they are listed. A folder is a `folder: Users/Admin` line on the request, not a nested
structure: the list stays flat, so reordering is a moved block and moving a request into
another folder is a one-line change.

```yaml
version: 1
name: Users
requests:
- id: 3f1a9c22-5f0e-4d51-9a7c-6b2e8d4f1a03
  name: List users
  method: GET
  url: '{{base_url}}/api/v1/users'
  query:
  - key: page
    value: '1'
    enabled: true
  - key: debug
    value: '1'
    enabled: false
  auth:
    type: bearer
    token: '{{secret:api_token}}'

- id: 9c4b7e10-2d83-4f6a-b105-7e39a2c8d514
  name: Create user
  folder: Admin
  method: POST
  url: '{{base_url}}/api/v1/users'
  headers:
  - key: Content-Type
    value: application/json
    enabled: true
  body:
    type: json
    content: |
      {
        "name": "Example",
        "email": "example@example.com"
      }
```

Design notes:

- **`enabled` is explicit per row.** Disabled parameters stay in the file rather than being
  deleted, so toggling one produces a one-word diff instead of a removed line.
- **Auth is structured, not a header.** `type: bearer` survives round-tripping and can be
  re-rendered correctly; a hand-written `Authorization` header cannot be reasoned about.
- **Secrets appear only as `{{secret:name}}` references.** Never values. See
  [secrets](secrets.md).
- **Bodies use block scalars**, so JSON stays readable and diffs line-by-line.
- **Defaults are omitted.** A request with no body and default transport settings writes
  neither, so a `GET` stays four lines rather than fifteen.
- **`id` is a stable identifier**, written once and never rewritten. Requests are referred to
  by name in the UI, but a name can be edited — the id is what history, open tabs and a
  save from the editor match on, so a rename updates the request rather than duplicating it.
- **Folders are paths, and only exist through the requests in them.** An empty folder has
  nothing to be written as, so it is not.
- **Transport settings are per request**, and `accept_invalid_certs` is *never* written at
  all. It lasts for the session that set it. Persisting it would hand the next person who
  clones the repository a request with certificate verification silently switched off.

## Environments

See [environments](environments.md) for variables and resolution order.

```yaml
version: 1
name: local
variables:
  base_url: http://localhost:8000
  api_version: v1
secrets:
  - api_token          # names only; values live in local/secrets.json
```

## Git-friendliness

The shared tier is meant to be reviewed in a pull request, which drives several choices:

- YAML with stable key ordering, so diffs reflect real edits rather than serializer churn
- No generated IDs in committed files where a name will do
- No timestamps in the shared tier — those belong to history
- One collection per file, so two people adding requests to different collections do not
  conflict

The intended payoff: someone clones the repo, opens RouteLens, and the project's requests are
already there.

## Standalone workspaces

A workspace with `kind: standalone` has no project attached and no discovery. The layout is
otherwise identical, and lives in an application data directory rather than in a repository.
