# Flows — a walkthrough

This builds a real flow, click by click, against the FastAPI fixture that ships in the
repository, then follows the run through the engine step by step so the behaviour you see
on the canvas is not magic. The finished flow is
[`examples/fastapi-user-lifecycle.yaml`](examples/fastapi-user-lifecycle.yaml); the
reference for every concept is [flows.md](flows.md).

What the flow proves:

```
health ──▶ create user ──▶ fetch it ──▶ if it is a new row ──true──▶ delete it ──▶ gone?
                                                            └─false──▶ (never runs)
list items ──▶ same item via /v2
```

The API under test lives in `tests/fixtures/fastapi/app/`. It keeps users in a dictionary
with three seed rows, guards `POST` and `DELETE` with a fixed bearer token (`letmein`), and
mounts the same items router under both `/v1` and `/v2`.

## 1. Start the API

```
cd tests/fixtures/fastapi
pip install -r requirements.txt
python -m uvicorn app.main:app --port 9000
```

`curl http://localhost:9000/health` should print `{"ok":true}`.

## 2. Open it in RouteLogic

**Open…** → choose `tests/fixtures/fastapi`. The scan runs on its own; the **Api** panel
lists 14 endpoints in five groups, two of them flagged with gaps (a prefix that comes from
`settings`, a router nobody mounts) — leave those alone, they are there on purpose.

Two things to set in the `{{ }}` dialog next to the environment picker:

| What | Value | Why |
|---|---|---|
| variable `base_url` | `http://localhost:9000` | the scan suggests it from the `Procfile`; check it |
| secret `api_token` | `letmein` | the fixture's bearer token — as a **secret**, so it never lands in a file |

## 3. Build the flow

**Flows → New flow.** An empty canvas, with a hint. Click **Api** to get the endpoint list
back. Each click below adds a card *wired after the selected one*, so the chain builds
itself; the new card is selected and its inspector opens on the right.

### Step 1 — `GET /health`

Click it. Card appears with `✓ 1 check` — the default `status < 400`.

In the inspector, **Assert** → **+ Body field**: path `ok`, operator `==`, expected `true`.
A JSON `true` is compared as the text `true`.

### Step 2 — `POST /api/v1/users/`

Click it (it wires after `health`). The card shows `Create a user` — the summary the scan
read off the decorator.

- **Request → Auth**: Bearer, token `{{secret:api_token}}`.
- **Request → Body**: JSON, `{"name": "Dana", "email": "dana@example.com"}`. The scan
  already knows the handler takes a `UserCreate` body, so the body tab is pre-selected.
- **Assert**: change the default check to `status == 201` — that is what the route
  declares, and `< 400` would also accept a 200 that the code never sends.
- **Extract → + Extract a value**: name `user_id`, from `body path`, path `id`.

The card now reads `↓ user_id · ✓ 1 check`.

### Step 3 — `GET /api/v1/users/{user_id}`

Click it. **Request → Params** shows the path parameter `user_id` with a red *required*
placeholder. Type `{{` — the popup offers `user_id`, because a card upstream extracts it.
Pick it. Add a header while here: **Headers** → `X-Trace-Id` = `flow-{{user_id}}`; the
handler echoes it back.

**Assert**: `+ Body field` → `name == Dana`, and another `+ Body field` →
`trace_id == flow-{{user_id}}`. Variables work on the right-hand side too.

### Step 4 — a condition

**+ Add → IF Condition**. It wires after the fetch. In the inspector: left `{{user_id}}`,
operator `>`, right `3`. The seed users are 1–3; anything we created is new.

### Step 5 — `DELETE /api/v1/users/{user_id}`

With the condition still selected, click the endpoint. It wires off the condition's
**true** output (the green handle). Path value `{{user_id}}`, **Auth** Bearer
`{{secret:api_token}}`, **Assert** `status == 204`.

### Step 6 — `GET /api/v1/users/{user_id}` again

Click it (wires after the delete). Path value `{{user_id}}`. **Assert**: *delete* the
default `status < 400` — a 404 is the point — and add `status == 404`, plus `+ Body field`
`detail contains no user`.

### Step 7 — the arm that never runs

Drag `GET /api/v1/users/` from the Api panel onto the canvas, below the delete. Then drag
from the condition's **false** handle (red) to the new card's left handle. This card exists
to show what an untaken branch looks like.

### Step 8 — a second, independent chain

Click empty canvas so nothing is selected, then click `GET /v1/items`: it lands as a new
root with no edge. **Params** → query `page` = `2`. **Extract** `first_item` from body path
`items[0].id`. **Assert** `+ Body field` `page == 2` and `+ Header`
`content-type contains application/json`.

Click `GET /v2/items/{item_id}` (wires after the list). Path value `{{first_item}}`.
**Assert** `+ Body field` `id == {{first_item}}`, `+ Body field` `via contains /v2/`, and
a duration check: source `duration`, `<`, `2000`.

### Save

Name it `user lifecycle` in the toolbar, **Ctrl+S**. It is now
`tests/fixtures/fastapi/.routelogic/flows/user lifecycle.yaml` and listed under **Flows**.

## 4. Run it

**Ctrl+Enter.** Cards light up one at a time: a pulsing accent border while running, then
green with `201 · 12 ms`, and so on. The bar above the canvas ends with
`Passed · 8 passed · 1 skipped · 96 ms · {{user_id}} {{first_item}}`.

The false arm is dashed: *skipped · is it a new row? went the other way*. Select it,
**Result** says the same in a sentence. Select `create user` → **Result**: the request as
sent (`POST http://localhost:9000/api/v1/users/` — the variable resolved, the auth header
applied), *Extracted* `{{user_id}}` = `4`, the checks with their actual values, and the
response body.

Run it again: `user_id` is `5`, the delete still hits 204, the 404 still holds. The condition
and the checks stay honest because the values come from the responses, not from the file.

Now break it. In `fetch it`, change the expected name to `Dan` and run. The card goes red:
`200 · 1 assertion failed`. Everything after it is dashed — *skipped · fetch it failed* —
and the edges out of it are red. The items chain, which does not depend on it, still runs
green. Hover the red card, click `↗`, and `app/api/users.py` opens at `get_user`.

## 5. What actually happened

The same run, from the engine's side. (Node ids are the ones in the example file.)

**Before anything is sent** — `Flow::validate`: every edge names a node that exists, the
condition's edges name outputs it has (`true`, `false`), no edge loops back. Then
`execution_order`: a topological sort of the edges, ties broken by the order the cards were
added. For this flow:

```
health → create → fetch → is_new → delete → gone → seed_only → items → item_v2
```

`items` is a root like `health`; it comes later only because it was added later. Where a
card sits on the canvas never enters into it.

**For each node, in that order**, the runner looks at the edges into it and decides
whether it runs:

| node | edges in | decision |
|---|---|---|
| `health` | none | run |
| `create` | `health` passed | run |
| `fetch` | `create` passed | run |
| `is_new` | `fetch` passed | run |
| `delete` | `is_new` passed, edge `true`, branch taken `true` | run |
| `gone` | `delete` passed | run |
| `seed_only` | `is_new` passed, edge `false`, branch taken `true` | **skip** — `branch_not_taken` |
| `items` | none | run |
| `item_v2` | `items` passed | run |

One *dead* edge (from a failed node, or from a node skipped because of a failure) is enough
to skip; one *live* edge is enough to run; edges from an untaken branch are neither. That
asymmetry is what lets both arms of a condition rejoin at a later card while a failure
still cuts everything below it.

**Running a request node** — say `fetch`:

1. Build the variable context: the `local` environment (`base_url`), the secret store
   (`api_token`), and — on top, taking precedence — everything extracted so far:
   `user_id = 4`.
2. Collect every `{{reference}}` in the request: URL, path values, headers, body, auth.
   Anything undefined fails the node *here*, listing all the missing names at once, before
   a byte goes out.
3. Resolve: `{{base_url}}/api/v1/users/{user_id}` with `user_id = {{user_id}}` becomes
   `http://localhost:9000/api/v1/users/4`; `X-Trace-Id: flow-4`.
4. Send through the same engine a request tab uses — no redirects unless asked, raw bytes
   kept, per-request timeout.
5. **Extract first**, then assert, so a failed check still shows what was pulled out. A
   body path that resolves to nothing is a failure (`could not extract {{x}}: nothing at
   body path …`); so is a body that is not JSON when a body path is asked for.
6. Evaluate each assertion: resolve its right-hand side (`flow-{{user_id}}` → `flow-4`),
   read the source from the response, compare. When both sides parse as numbers the
   comparison is numeric — `status == 201` does not care that one side is text.
7. Outcome: passed if it was sent, every extraction found a value, and every check holds.
   Extracted values are committed to the run's variables **only on pass**.
8. Record the exchange to history, redacted — the token in the `Authorization` header is
   masked before it is written, and so would a token echoed back in a response body.

**Running a condition** — `is_new`: resolve both sides (`4` and `3`), compare with `>`
numerically, take `true`. The node passes either way; only the branch differs.

**Events** — each of those steps was reported as it happened: `started` with the order,
`node_started`, `node_finished` with the full result, `finished` with the whole run. The
canvas is subscribed to that stream, which is why cards change one at a time rather than
all at the end, and why the summary can appear while the last card is still settling.

**The file** — what was saved is exactly what ran, minus the secret's value:

```yaml
  - id: fetch
    name: fetch it
    type: request
    request:
      spec_ref: GET /api/v1/users/{}
      method: GET
      url: "{{base_url}}/api/v1/users/{user_id}"
      path_values:
        user_id: "{{user_id}}"
      headers:
        - key: X-Trace-Id
          value: flow-{{user_id}}
    assert:
      - from: body
        path: name
        op: equals
        expected: Dana
```

`spec_ref` is the endpoint's identity from the scan — method plus path *shape* — which is
what the `↗` button resolves to a file and line, and what survives the handler being renamed
or moved.

## 6. Things worth trying next

- **Rename a variable** upstream and watch the downstream card fail with
  `undefined variable(s): user_id` before sending anything.
- **Swap the environment** to one whose `base_url` points at a server that is down: the
  first card fails with the transport error, everything else is skipped, and the items
  chain — a separate root — fails on its own.
- **Reorder cards on the canvas** and run again: nothing changes. Then reverse an edge
  and the toolbar refuses to run until the cycle is gone.
- **Check history**: every request the run sent is listed, newest first, with the token
  masked.
- Run the same flow from the test suite:
  `ROUTELOGIC_FIXTURE_URL=http://localhost:9000 cargo test -p rl-core --test fixture_flow`.
