//! # rl-discovery
//!
//! Reads a project's source and produces [`rl_model::EndpointSpec`]s. It never executes the
//! project's code — that is [runtime enrich][enrich], which lives behind an explicit opt-in.
//!
//! ## Pipeline
//!
//! ```text
//! ProjectDetector → FrameworkDetector → SourceIndex → RegistrationGraph
//!                 → ConstantResolver → SchemaExtractor → BaseUrlInference
//! ```
//!
//! ## The one idea worth protecting
//!
//! Framework adapters do **not** resolve paths. They recognise three facts — *this creates a
//! router*, *this registers a route*, *this mounts a router at a prefix* — and the shared
//! registration graph composes full paths from them.
//!
//! That is what keeps adapters small enough to add cheaply, and why prefix resolution is
//! written once rather than once per framework. If an adapter starts joining paths itself,
//! the abstraction has sprung a leak.
//!
//! [enrich]: https://github.com/AryanMahajan/routelens/blob/main/docs/discovery/runtime-enrich.md
//!
//! Status: not yet implemented. Scheduled for P3 (core + FastAPI), P4 (Next.js, Express),
//! P5 (Flask), P6 (Django).

#![forbid(unsafe_code)]
