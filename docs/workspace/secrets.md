# Secrets

Git-friendly storage plus bearer tokens is exactly how credentials end up committed. The
original RouteLens specification did not mention secrets at all, which is why they are
designed in from the start rather than bolted on later.

The rule is simple: **secret values never enter a file that could be committed.**

## How it works

An environment declares secret *names*. The committed file contains the names and nothing
else:

```yaml
# .routelens/environments/local.yaml   — committed
version: 1
name: local
variables:
  base_url: http://localhost:8000
secrets:
  - api_token
  - db_password
```

Requests reference them explicitly:

```yaml
auth:
  type: bearer
  token: "{{secret:api_token}}"
```

Values live in the private tier:

```
.routelens/local/secrets.json      # gitignored
```

or, preferably, in the OS keychain — Credential Manager on Windows, Keychain on macOS,
Secret Service on Linux — with only a reference stored locally.

`.routelens/.gitignore` is written automatically when the workspace is created, so `local/`
is excluded from the moment it exists.

## The `secret:` prefix is deliberate

`{{secret:api_token}}` is more verbose than `{{api_token}}`, on purpose. The prefix makes
secret usage visible when reading a committed file, so a reviewer can see at a glance which
requests carry credentials. It also means a secret can never be referenced *accidentally* by
a name collision with an ordinary variable.

## Redaction

Secret values are redacted at every point where data leaves the private tier:

| Surface | Behaviour |
|---|---|
| Saved requests | Stored as `{{secret:name}}` references, never resolved values |
| History entries | Resolved secret values replaced with `••••••` before writing |
| Exported cURL | Reference preserved by default; resolving requires explicit confirmation |
| Generated documentation | Never includes values |
| Logs and error messages | Redacted |
| Crash reports | Private tier is never included |

Resolution happens as late as possible — in the HTTP engine, immediately before the request
is sent — so the window in which a plaintext secret exists is as small as it can be.

## Captured tokens

Values captured from a response (see [environments](environments.md)) default to
`scope: session`: held in memory for the current session and never written to disk. This is
the right default for short-lived access tokens, which are the common case.

`scope: environment` persists a captured value, and it is treated as a secret from then on.

## What RouteLens does not do

Stated plainly so the boundary is clear:

- It does not encrypt `secrets.json` beyond filesystem permissions when the OS keychain is
  unavailable. If keychain access is not available on your platform, treat that file with the
  same care as a `.env`.
- It does not sync secrets anywhere. There is no cloud component.
- It does not read your existing `.env` files as a secret source unless you explicitly import
  from one.

## Checklist for sharing a workspace

Before committing `.routelens/` for the first time:

- [ ] `.routelens/.gitignore` exists and ignores `local/`
- [ ] `git status` shows no `local/` contents
- [ ] No literal tokens in `collections/*.yaml` — search for `Bearer ` and `api_key`
- [ ] Environment files list secret *names* only
- [ ] `local.yaml` has working non-sensitive defaults, so a fresh clone runs
