//! # rl-import
//!
//! Turns pasted or imported API definitions into [`rl_model`] types.
//!
//! - **cURL** — a real shell tokenizer first, flag parsing second. Commands copied from
//!   browser devtools are full of quoting that a naive whitespace split mangles.
//! - **OpenAPI** — 3.1, 3.0, and Swagger 2.0, with local `$ref` resolution.
//! - **Raw HTTP** — request line, headers, blank line, body.
//!
//! The guiding rule: the user should never have to decide by hand whether a pasted value
//! belongs in headers, query, auth, or body.
//!
//! Auth is always *structured* rather than left as a raw `Authorization` header, because that
//! is what lets a request round-trip — an environment switch can swap the token, and export
//! can re-render the header correctly.
//!
//! ## Why the OpenAPI importer is built early
//!
//! Runtime enrich reuses it verbatim: `app.openapi()` output is just another OpenAPI
//! document. Building it once in P2 makes the P5 runtime path nearly free.
//!
//! Status: not yet implemented. Scheduled for P2.

#![forbid(unsafe_code)]
