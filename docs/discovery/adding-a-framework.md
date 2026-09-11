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
    fn extract(&self, file: &ParsedFile, sink: &mut FactSink);
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

Walk the parsed tree and push facts into the sink. Three facts describe routing; two more
let the graph link them across files.

```rust
sink.router(RouterFact {
    symbol,                  // SymbolId: (module, variable name)
    prefix,                  // PathTemplate, possibly containing Unresolved
    group,                   // tags / name, used for tree grouping
    is_app_root,             // true for `app = FastAPI()`, `app = express()`
    span,
});

sink.route(RouteFact {
    router,                  // SymbolRef: the name as written, e.g. `users.router`
    methods,                 // one or more
    path,                    // PathTemplate relative to its router
    query_params, headers,
    body, auth,              // best-effort; None is always acceptable
    span,                    // becomes SourceLocation
    ..
});

sink.mount(MountFact {
    parent, child,           // SymbolRefs, as written at the mount site
    prefix,
    group,
    auth,                    // a guard on the mount, inherited by every route beneath
    span,
});

sink.import(ImportFact {     // how `child` above might be traced to another file
    module, local_name,
    source,                  // in the language's own convention
    original,                // the name in the source module; `default` for JS defaults
    level,                   // Python relative-import dots
});

sink.export(ExportFact {     // JavaScript only: `module.exports = router`
    module, exported, local,
});
```

A `SymbolRef` is deliberately *unresolved*: `include_router(router)` might name a local
variable or an import, and only the graph — which sees every file's imports and exports —
knows which. The graph follows re-exports, submodule imports and CommonJS defaults; the
adapter just reports what was written.

Frameworks without routers — Next.js, whose paths come from the filesystem — emit `route`
facts against one virtual app root per app, attached with an `ImportFact` whose `source`
starts with `/` (project-root-relative, already resolved). The graph is then flat, and the
resolver handles that case without special-casing.

### Shared helpers

Recognition code that is about the *language* rather than the framework lives in
`adapters/python.rs` and `adapters/js.rs`: constant folding, call arguments, import and
export collection, and — for JavaScript — what a handler reads off its request object.
A Flask adapter reuses everything the FastAPI one does; a Koa or Fastify adapter would
reuse everything Express does.

### Emitting unknowns

When a value cannot be determined statically, emit `PathSegment::Unresolved { expr }` with
the source text. Never guess, and never silently drop the route. A visible gap is useful; a
confidently wrong path is worse than no path at all.

## Steps to add one

1. **Add the tree-sitter grammar** if the language is not already indexed.

2. **Create `crates/rl-discovery/src/adapters/<name>.rs`** and implement the trait.

3. **Write the recognisers.** The existing adapters walk the tree with `index::walk` and
   match on a handful of node kinds; keep each recogniser small and named after the
   syntax it recognises (`extract_decorated`, `mount`, `method_exports`). Dump a sample
   file's tree with `cargo run -p rl-discovery --example sexp -- file.ts` to see the node
   kinds before writing one.

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
