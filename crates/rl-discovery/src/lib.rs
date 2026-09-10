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
//! Status: P3 complete — project detection, source index, registration graph, constant
//! folding, the FastAPI adapter, and base URL inference. Next.js and Express arrive in P4,
//! Flask in P5, Django in P6.

#![forbid(unsafe_code)]

pub mod adapters;
pub mod baseurl;
pub mod error;
pub mod facts;
pub mod graph;
pub mod index;
pub mod project;
pub mod scan;

pub use adapters::{Detection, FrameworkAdapter};
pub use baseurl::BaseUrlCandidate;
pub use error::{DiscoveryError, Result};
pub use facts::{
    FactSink, ImportFact, MountFact, RouteFact, RouterFact, Span, SymbolId, SymbolRef,
};
pub use graph::{GraphWarning, RegistrationGraph, ResolvedRoute};
pub use index::{ParsedFile, SourceIndex};
pub use project::{Language, ProjectContext};
pub use scan::{scan, scan_project, DetectedFramework, ScanResult, ScanStats};
