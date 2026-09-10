# Concepts

RouteLens has one internal model. Source code, cURL, OpenAPI, raw HTTP, and manual entry all
resolve into it, so nothing downstream needs to know where a request came from.

```
Source Code ──┐
cURL ─────────┤
OpenAPI ──────┼──→  Unified model  ──→  UI · HTTP engine · storage
Raw HTTP ─────┤
Manual ───────┘
```

## The two core types

The most important decision in the model is that there are **two** types, not one.

A route discovered in source is a *template*: it has a path shape, a schema, and a source
location, but no host and no values. A saved request is *executable*: it has concrete values
and a real URL, but no source location. Collapsing these into a single type looks tempting
early and becomes painful quickly — history entries acquire meaningless source fields,
discovered routes acquire meaningless value fields, and every consumer has to check which
kind it actually has.

### `EndpointSpec` — discovered, template

What a project *exposes*.

| Field | Meaning |
|---|---|
| `method` | HTTP method |
| `path_template` | Segment list, not a string — see below |
| `path_params` | Name, type hint, required |
| `query_params` | Name, type hint, default, required |
| `headers` | Expected/required headers |
| `body` | Request body schema, where detectable |
| `auth` | Authentication requirement, where detectable |
| `source` | File, line, column of the definition |
| `origin` | `StaticScan{framework}` · `Runtime` · `OpenApi` · `Manual` |
| `group` | Tag, router name, or folder — drives the tree |
| `confidence` | How sure discovery is |

### `RequestDraft` — executable

What you actually *send*.

| Field | Meaning |
|---|---|
| `spec_ref` | Optional link back to the `EndpointSpec` it came from |
| `method` | HTTP method |
| `url` | May contain `{{variables}}` |
| `path_values` | Concrete values for path parameters |
| `query` / `headers` / `cookies` | Key-value pairs, each individually enable-able |
| `auth` | Concrete auth config |
| `body` | `None` · `Json` · `Form` · `Multipart` · `Raw` · `Binary` |

Collections and history store drafts. The explorer tree shows specs. Opening a spec produces
a draft.

## Paths are segments, not strings

A path template is a list of segments:

```rust
enum PathSegment {
    Literal(String),                                  // "users"
    Param { name: String, ty: Option<TypeHint> },     // {user_id}
    Unresolved { expr: String },                      // settings.API_PREFIX
}
```

`Unresolved` is the load-bearing one. When RouteLens cannot statically determine a segment —
a prefix read from settings, a value computed at import time — it records the source
expression and marks the segment unresolved. The UI displays the gap.

This is deliberate. A tool that silently guesses a wrong path is worse than one that admits
it does not know, because a wrong path fails in a way you will blame on your own code.

## Origin and confidence

Every spec records how it was learned:

- **`StaticScan`** — read from source without executing anything. Always available, sometimes
  incomplete.
- **`Runtime`** — obtained by importing the app and asking it directly. Highest fidelity,
  requires opt-in. See [runtime enrich](discovery/runtime-enrich.md).
- **`OpenApi`** — imported from a specification document.
- **`Manual`** — you typed it.

When both static and runtime results exist for the same route, they are merged rather than
one replacing the other: runtime wins on schemas and auth, static wins on source location, so
you keep both accuracy and code navigation.

## Variables

Anywhere a value is accepted, `{{name}}` interpolation works. Resolution order is:

```
request-local  →  environment  →  workspace globals  →  secrets
```

One resolver implementation serves both the UI preview and the HTTP engine, so the request
you see previewed is byte-for-byte the request that gets sent.

See [environments](workspace/environments.md) and [secrets](workspace/secrets.md).
