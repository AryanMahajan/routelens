//! # rl-http
//!
//! Request execution. Built on `reqwest`, and configured against most of its conveniences.
//!
//! An API client's job is to send *exactly* what the user described, including the things a
//! normal HTTP client would helpfully correct. Convenience is the wrong default here:
//!
//! - Redirects are **not** followed unless asked; when they are, the chain is shown.
//!   Beyond showing what the server actually returned, this avoids silently forwarding an
//!   `Authorization` header to a host the user did not intend to contact.
//! - Raw response bytes are preserved alongside the decoded body — no silent decompression.
//! - Header order is preserved, and duplicate or unusual headers pass through.
//! - Certificate verification is toggleable **per request**, never globally, so relaxing it
//!   for one call against localhost cannot weaken a later call to production.
//! - Response bodies stream: the in-UI preview is capped, large payloads go to a file.
//! - Timing breaks down into DNS, TCP, TLS, TTFB, and total.
//!
//! Variables are resolved as late as possible — here, immediately before sending — so the
//! window in which a plaintext secret exists is as small as it can be. See
//! [`rl_model::VariableContext`].
//!
//! Status: not yet implemented. Scheduled for P1.
//!
//! Known risk: the timing breakdown needs a custom connector. If that proves fiddly, TTFB
//! and total ship first and the breakdown is refined afterwards — it must not block P1.

#![forbid(unsafe_code)]
