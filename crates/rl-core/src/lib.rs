//! # rl-core
//!
//! The facade. Owns application state, orchestrates the other crates, and exposes one
//! coherent API.
//!
//! ```text
//! rl-model  ←──  rl-discovery
//!     ↑     ←──  rl-import
//!     │     ←──  rl-http
//!     │     ←──  rl-workspace
//!     │               │
//!     └────  rl-core ─┘
//!                │
//!           src-tauri      ← command wrappers only, no logic
//!                │
//!               ui
//! ```
//!
//! ## The boundary this crate exists to hold
//!
//! `src-tauri` contains **no logic** — only command wrappers, event emission, and filesystem
//! scope handling. Everything it calls lives here.
//!
//! That keeps discovery testable with a plain `cargo test` against fixture repositories,
//! with no GUI harness in the loop, which matters because discovery is the highest-risk
//! component and needs dozens of fixture projects exercised against it.
//!
//! It also means a second shell — a CLI, for use over SSH or in a dev container — would be a
//! small binary over an existing library rather than a rewrite. That is a side benefit of
//! the boundary, not a goal of the project.
//!
//! Status: not yet implemented. Grows through every phase.

#![forbid(unsafe_code)]
