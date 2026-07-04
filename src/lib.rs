//! `rejolt` — the routed-memory reseed engine.
//!
//! This is the engine **library**. The `rejolt` binary (`src/main.rs`) is a
//! thin entry point over it, and the integration tests in `tests/` plus every
//! later build packet consume the same library surface. Keeping the engine in
//! a lib (not buried in `main.rs`) is what lets conformance checks and later
//! packets link against it directly.
//!
//! WP-0 ships only the skeleton: the crate version surface, the CLI shape
//! (nine D20 subcommands + `hook` mode), and the G2 conformance self-test
//! harness. Real behavior lands in later packets, which add their own modules
//! rather than filling stubs guessed here.

pub mod bench;
pub mod bootstrap;
pub mod catalog;
pub mod cli;
pub mod config;
pub mod conformance;
pub mod curation;
pub mod frontmatter;
pub mod grammar;
pub mod guard;
pub mod hook;
pub mod hooks;
pub mod index;
pub mod normalize;
pub mod path_class;
pub mod projection;
pub mod rebuild;
pub mod recall;
pub mod tag;
pub mod telemetry;
pub mod tier;

/// The crate version, sourced from Cargo at compile time. Surfaced by
/// `rejolt --version` and asserted against by the CLI smoke test so the binary
/// and library never disagree about their version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
