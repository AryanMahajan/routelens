# Environments and variables

An environment is a named set of variables. Switching environments repoints every request in
the workspace without editing any of them.

## Using variables

`{{name}}` works anywhere a value is accepted — URL, query values, header values, body
content, and auth fields.

```
GET {{base_url}}/api/{{api_version}}/users/{{user_id}}
```

## Resolution order

Highest priority first:

```
1. request-local     values set on the request itself
2. environment       the active environment
3. workspace globals shared across environments
4. secrets           the private tier
```

The first match wins. A missing variable is a **hard error**, not an empty string — a request
to `https:///api/users` fails in a confusing way, so RouteLens refuses to send it and tells
you which variable is unset.

One resolver implementation serves both the UI preview and the HTTP engine. The preview is
therefore not an approximation: what it shows is what gets sent.

## Environment files

```yaml
version: 1
name: local
variables:
  base_url: http://localhost:8000
  api_version: v1
  user_id: "42"
secrets:
  - api_token
```

`variables` are committed and visible. `secrets` lists **names only** — values live in the
private tier and are referenced as `{{secret:api_token}}`. See [secrets](secrets.md).

## Base URL

For a project workspace, `base_url` is usually populated from discovery. RouteLens infers
candidates from run scripts, `.env` files, Dockerfiles, and compose files, then offers them
ranked. Whatever you pick is written into the active environment as an ordinary variable, so
it stays visible and editable rather than hidden in tool state.

## Chaining requests

Values can be captured out of a response into a variable, which covers the common
authenticate-then-call pattern:

```yaml
- name: Login
  method: POST
  url: "{{base_url}}/api/v1/login"
  body:
    type: json
    content: |
      {"username": "{{username}}", "password": "{{secret:password}}"}
  capture:
    - from: body
      path: $.access_token
      into: token
      scope: session      # session | environment
```

Subsequent requests use `{{token}}`.

`scope: session` keeps the captured value in memory for the current session only — the right
default for short-lived tokens, since it means they are never written to disk at all.
`scope: environment` persists it into the private tier, treated as a secret.

## Conventions

- Commit a `local.yaml` with working defaults so a fresh clone runs immediately.
- Keep non-sensitive configuration in `variables`, where reviewers can see it change.
- Anything that would be awkward in a public diff belongs in `secrets`.
- Prefer `{{base_url}}` over hardcoded hosts in every saved request, so environment switching
  actually works.
