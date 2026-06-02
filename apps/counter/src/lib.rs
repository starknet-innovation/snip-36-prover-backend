//! Counter — the reference application, and the home of the SDK's *generic*
//! CLI commands.
//!
//! Besides the Counter contract demo (routes, selectors, e2e), this crate backs
//! the workspace-wide `snip36 health` and `snip36 e2e` commands (wired up in
//! `snip36-cli`'s `main.rs`). They live here because Counter is the canonical
//! end-to-end example — the name "counter" does NOT mean they are
//! counter-specific. If you're looking for the default health check or e2e
//! flow, this is it.

pub mod e2e;
pub mod health;
pub mod routes;
pub mod selectors;
