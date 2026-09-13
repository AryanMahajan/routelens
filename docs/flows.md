# Flows

A flow is a multi-step API test built on a canvas: requests from the project's discovered
API, wired together so that what one response returns feeds the next request, with checks
along the way. It runs as one test, shows exactly where it broke, and every card can jump
to the handler that serves it.

```
POST /auth/login ──▶ GET /me ──▶ GET /users/{{user_id}}
  ↓ auth_token        ↓ user_id      ✓ status < 400
  ✓ status < 400      ✓ status < 400 ✓ body.name == Aryan
```

## Building one

Open a project, scan it, then **Flows → New flow**. Add steps three ways:

- **click** an endpoint in the API panel — it becomes a card, wired after the selected one;
- **drag** an endpoint from the API panel onto the canvas — it lands where you drop it,
  unconnected;
- **+ Add** in the toolbar — the same endpoint list with a filter, plus a blank request
  and a condition.

Connect cards by dragging from a right-hand handle to a left-hand one. Edges are
*dependencies*: a card runs after everything wired into it, and only if those passed.

| Action | How |
|---|---|
| Pan · zoom | drag the background · scroll or pinch |
| Move | drag a card (Shift+drag for a box selection, Ctrl+click to add to it) |
| Delete | select, then `Delete` or `Backspace` |
| Duplicate | `Ctrl+D` |
| Select all · none | `Ctrl+A` · `Esc` |
| Run | `Ctrl+Enter` or **▶ Run** |
| Save | `Ctrl+S` or **Save** |

## What a card is

A request card is an ordinary RouteLens request — the same editor a request tab has — plus
two things that make it a test step:

**Extract** — pull values out of the response for later steps. Each becomes a
`{{variable}}` for everything downstream, taking precedence over an environment variable
of the same name. Sources: a body path (`access_token`, `user.id`, `items[0].name`), a
header, the status, the raw body, or the duration. Scalars are extracted as plain text;
objects and arrays as compact JSON, so a whole record can be re-sent as a body.

**Assert** — what must hold for the step to pass. Every new card starts with
`status < 400`. Operators: `==`, `!=`, `contains`, `not contains`, `exists`, `not exists`,
`>`, `<`. When both sides are numbers the comparison is numeric, so `200 == "200"` and
`9 < 10`. The right-hand side may use `{{variables}}`.

A **condition** card compares two interpolated values and sends the run down its `true` or
`false` output. Whatever hangs off the other output is *skipped*, not failed.

## How a run proceeds

Steps run one at a time in dependency order — a topological sort of the edges, with ties
broken by the order cards were added, never by where they sit on the canvas. Moving a card
cannot change what the test does.

Before each step the runner looks at the edges into it:

- an edge from a step that **failed** — or was skipped because something upstream of it
  failed — is *dead*, and one dead edge is enough to skip the step. A step whose
  prerequisite did not happen must not run against half-set-up state.
- an edge out of a condition's untaken output is *inactive*. A step runs as long as **one**
  of its edges is live, so the two arms of a condition can rejoin.
- a step with no edges into it always runs.

A request step **fails** when an assertion does not hold, an extraction finds nothing, a
`{{variable}}` it needs is undefined (reported before anything is sent), or the request
never gets a response. It **passes** otherwise — a 404 with no assertion against it is
not a failure, which is what lets `DELETE … → GET … → assert status == 404` work.

Extracted values are committed only when the step passed.

## Reading a failure

The canvas is the report: passed cards are green, the failed card is red with its reason,
skipped cards are dashed and say which step's failure kept them from running, and the
edges the run took are green while the ones a failure cut are red. The bar above the
canvas totals it up and lists every variable the run extracted.

Select a card and open **Result** to see what happened to it: the request as it was
actually sent, the values it extracted, each check with what was expected and what was
found, and the full response — body, headers, timing — in the same viewer a request tab
uses.

Every request a flow sends is recorded in history, redacted, exactly like a single send.

## Source

A card built from a discovered endpoint keeps the endpoint's identity (`spec_ref`), so the
`↗` on the card and in the inspector opens the handler in your editor — the same jump the
API panel offers. If the project has not been scanned in this session the button is
absent until it has.

## On disk

Flows are saved to `.routelens/flows/<name>.yaml` beside the project's environments, so
they are committed with the project and whoever clones it gets the tests. The file is a
flat list of cards, each with its request, `extract` and `assert`, plus the edges — meant
to be readable in a pull request.

```yaml
name: login smoke
nodes:
  - id: 3f1c…
    type: request
    position: { x: 80, y: 120 }
    request: { method: POST, url: "{{base_url}}/auth/login", … }
    extract:
      - name: auth_token
        from: body
        path: access_token
    assert:
      - from: status
        op: less_than
        expected: "400"
edges:
  - from: 3f1c…
    to: 9a2e…
```

Secret values never appear: a request references `{{secret:name}}` and the value is
substituted moments before sending, as everywhere else in RouteLens.

## Not yet

- Loops, retries, delays, parallel branches — this is a test, not a workflow engine.
- Flow-level input variables (use the environment).
- Assertions with a regular expression.
- Cancelling a run in progress.
- Persisting run history as a unit (each request is in history; the run as a whole is not).
