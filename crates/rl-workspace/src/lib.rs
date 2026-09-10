//! # rl-workspace
//!
//! Everything RouteLens puts on disk, split into three tiers by how the data should be
//! treated.
//!
//! ```text
//! .routelens/
//! ├── workspace.yaml       committed
//! ├── collections/*.yaml   committed
//! ├── environments/*.yaml  committed — secret NAMES only, never values
//! ├── .gitignore           committed — written automatically, ignores local/
//! └── local/               never committed
//!     ├── secrets.json     private   — or an OS keychain reference
//!     ├── history.sqlite   private
//!     └── index.sqlite     disposable — rebuildable source index cache
//! ```
//!
//! ## Why three tiers and not two
//!
//! "Committed" and "not committed" leaves nowhere to express the difference between *a
//! secret you must protect* and *a cache you can delete*. Conflating them means either
//! caches get backed up or secrets get treated as disposable.
//!
//! | Tier | Committed | Safe to delete |
//! |---|---|---|
//! | Shared — collections, environments | yes | no, it is real work |
//! | Private — secret values | no | no, they are real credentials |
//! | Disposable — history, index | no | yes, it rebuilds |
//!
//! `.routelens/.gitignore` is written at workspace creation, not after someone notices. Git
//! friendliness plus bearer tokens is exactly how credentials reach version control.
//!
//! Serialization uses stable key ordering so diffs reflect real edits rather than serializer
//! churn — these files are meant to be reviewed in a pull request.
//!
//! Status: not yet implemented. Scheduled for P0 (layout, secrets) and P1 (history).

#![forbid(unsafe_code)]
