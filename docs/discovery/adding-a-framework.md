# Adding a framework

Design principle #6 says framework support is implemented through independent adapters rather
than framework-specific logic scattered through the app. This document is the test of that
claim: if adding a framework cannot be described as a short contract, the abstraction is
wrong and the *code* should change, not this page.

## What an adapter is responsible for

An adapter recognises framework-specific syntax and emits framework-agnostic facts. That is
all. In particular, an adapter **does not**:

- resolve prefixes or compose full paths — the registration graph resolver does that
- read files or manage the parse cache — the source index does that
- fold constants — the constant resolver does that
- know anything about the UI, the HTTP engine, or storage

Keeping adapters this thin is what makes them cheap to add and safe to get wrong.

## The contract

```rust
pub trait FrameworkAdapter {
    /// Stable identifier, e.g. "fastapi".
    fn id(&self) -> &'static str;

    /// Languages whose files this adapter wants to see.
    fn languages(&self) -> &[Language];

    /// How strongly this project looks like this framework.
    fn detect(&self, project: &ProjectContext) -> Detection;

    /// Which files are worth parsing at all.
    fn candidate_files(&self, project: &ProjectContext) -> Vec<PathBuf>;

    /// Emit facts from one parsed file. The core does the rest.
    fn extract(&self, file: &ParsedFile, sink: &mut FactSink) -> Result<()>;
}
```

### `detect`

Return a score plus the evidence behind it. Manifest dependencies are weak evidence; actual
import statements are strong. Several adapters may match one project, and that is not an
error — a repo can legitimately be both a Next.js app and an Express server.

### `candidate_files`

A cheap filter, not an analysis. Narrow by extension and by convention (`app/api/**`,
`**/urls.py`, files importing the framework). Anything excluded here is never parsed, so this
is the main lever on scan speed.

### `extract`

Walk the parsed tree — normally with tree-sitter queries — and push facts into the sink:

```rust
sink.router(RouterFact {
    symbol,                  // the variable the router is bound to
    prefix,                  // PathTemplate, possibly containing Unresolved
    group,                   // tags / name, used for tree grouping
    span,
});

sink.route(RouteFact {
    router: symbol,          // which router this attaches to
    methods,                 // one or more
    path,                    // PathTemplate relative to its router
    params, body, auth,      // best-effort; None is always acceptable
    span,                    // becomes SourceLocation
});

sink.mount(MountFact {
    parent,                  // router being mounted onto
    child,                   // router being mounted
    prefix,
    span,
});

sink.app_root(symbol);       // where path resolution starts
```

Frameworks without routers — Next.js, whose paths come from the filesystem — simply emit
`route` facts against a single synthetic app root. The graph is then flat, and the resolver
handles that case without special-casing.

### Emitting unknowns

When a value cannot be determined statically, emit `PathSegment::Unresolved { expr }` with
the source text. Never guess, and never silently drop the route. A visible gap is useful; a
confidently wrong path is worse than no path at all.

## Steps to add one

1. **Add the tree-sitter grammar** if the language is not already indexed.

2. **Create `crates/rl-discovery/src/adapters/<name>.rs`** and implement the trait.

3. **Write the queries.** Keep route-recognition patterns as tree-sitter queries in the
   adapter module rather than hand-written visitor code, so the patterns stay readable and
   reviewable.

4. **Add a fixture project** under `tests/fixtures/<name>/`. A realistic small app, not a toy:
   routes split across files, at least one nested mount, at least one prefix constant.

5. **Add the deliberately nasty cases.** Every fixture should include, where the framework
   allows it:
   - a router mounted at two different prefixes
   - a router declared but never mounted
   - a prefix that cannot be resolved statically
   - a route registered dynamically, which you expect to *miss*

   Recording expected misses is as valuable as recording expected hits — it documents the
   boundary of the adapter and turns silent regressions into failing tests.

6. **Snapshot the expected output** as `tests/fixtures/<name>/expected-routes.json` and wire
   it into the snapshot test harness.

7. **Update [frameworks.md](frameworks.md)** with what is recognised and — importantly — what
   is not.

## Review checklist

- [ ] No path composition inside the adapter
- [ ] No file I/O inside the adapter
- [ ] Unresolvable values emitted as `Unresolved`, never guessed
- [ ] Unmounted routers emitted, never dropped
- [ ] Fixture covers nested mounts, double mounts, and unresolved prefixes
- [ ] Expected misses documented in the snapshot
- [ ] `frameworks.md` updated with both capabilities and gaps
