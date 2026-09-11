# Import

Three importers, all producing the same unified model. The guiding rule: **you should never
have to decide by hand whether a pasted value belongs in headers, query, auth, or body.**

## cURL

Paste a command, get a structured request.

```bash
curl 'https://api.example.com/users?page=2' \
  -H 'Authorization: Bearer token' \
  -H 'Content-Type: application/json' \
  --data-raw '{"name":"Aryan"}'
```

becomes

```
Method   POST                          (inferred: body present, no -X)
URL      https://api.example.com/users
Query    page = 2                      (split out of the URL)
Headers  Content-Type = application/json
Auth     Bearer  token                 (recognised, not left as a raw header)
Body     JSON  { "name": "Aryan" }
```

### Two-stage parsing

Import runs a real shell tokenizer before it looks at any flags. This matters more than it
sounds: cURL commands copied from browser devtools are full of single quotes, embedded
double quotes, line continuations, and `$'...'` escapes, and a naive split on whitespace
mangles them.

1. **Tokenize** — POSIX shell word splitting: single quotes, double quotes with escapes,
   `$'...'` ANSI-C quoting, backslash line continuations, comments. Chrome's
   **"Copy as cURL (cmd)"** on Windows is recognised automatically and split by `cmd.exe`
   rules instead: `^"` quotes, `^%` / `^$` escapes, `^` line continuations, and `^\^"`
   for a quote inside a quoted argument.
2. **Parse flags** — map tokens onto request fields.

In the app there is no import step for cURL at all: paste the command into the URL bar
and it becomes the request.

### Supported flags

| Flag | Effect |
|---|---|
| `-X`, `--request` | Method |
| `-H`, `--header` | Header |
| `-d`, `--data`, `--data-raw` | Body; implies POST |
| `--data-binary` | Body, unmodified |
| `--data-urlencode` | Body, URL-encoded |
| `-F`, `--form` | Multipart body |
| `-u`, `--user` | Basic auth |
| `-b`, `--cookie` | Cookies |
| `-G`, `--get` | Moves data into the query string |
| `-k`, `--insecure` | Disables certificate verification for this request |
| `--compressed` | Accept-Encoding |
| `-L`, `--location` | Follow redirects |
| `--url` | URL, when given as a flag |
| `-A`, `--user-agent` | User-Agent header |
| `-e`, `--referer` | Referer header |
| `-X` with no body | Method only |

Unrecognised flags are reported rather than silently dropped, so an import that lost
something tells you what.

### Inference rules

- Method defaults to `GET`, or `POST` when a body flag is present and no `-X` is given.
- Query parameters are split out of the URL into structured rows.
- `Authorization: Bearer x` becomes structured bearer auth, not a raw header.
- `Authorization: Basic <base64>` is decoded into basic auth.
- `-u user:pass` becomes basic auth.
- Body type is detected from `Content-Type` first, then by sniffing the content.

Structuring auth rather than leaving it as a header is what lets the request round-trip: an
environment switch can swap the token, and export can re-render the header correctly.

## OpenAPI

Import a specification and get a collection.

**Supported** — OpenAPI 3.1, OpenAPI 3.0, and Swagger 2.0 (converted on the way in).

**Handled**

- Local `$ref` resolution, including recursive schemas
- `servers` become base URL candidates, and populate the environment
- `securitySchemes` become structured auth configurations
- Path, query, header, and cookie parameters with types and defaults
- Request body schemas, with the first example or a generated skeleton pre-filled
- `tags` become the collection folder structure
- `operationId` and `summary` become request names

**Not handled**

- Remote `$ref` resolution across the network — only local and same-document references
- `callbacks` and `links`
- `discriminator`-based polymorphism reduces to the base schema

This importer is deliberately built early, in P2, because
[runtime enrich](discovery/runtime-enrich.md) reuses it verbatim: `app.openapi()` output is
just another OpenAPI document. Building it once serves both features.

## Raw HTTP

Paste a raw request, from a log, a proxy, or a `.http` file:

```http
POST /api/v1/users HTTP/1.1
Host: api.example.com
Content-Type: application/json
Authorization: Bearer token

{"name": "Aryan"}
```

Parsed as request line, headers, blank line, body. The URL is reconstructed from `Host` plus
the request target, and the same auth-structuring rules as cURL apply.

## Export

Import is not one-way. Any request can be exported as cURL, raw HTTP, or an OpenAPI operation.

By default, exported output keeps `{{secret:name}}` references rather than resolving them.
Exporting with real credentials requires explicit confirmation — see
[secrets](workspace/secrets.md).
