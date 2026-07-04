//! Command-line surface for `rejolt` (D20): one multiplexed binary exposing
//! nine subcommands plus a `hook` entry mode.
//!
//! WP-0 pins only the **shape** — the subcommand set and the frozen `hook`
//! event set (D19). The authoritative per-command flag / output / exit
//! contract is plan Appendix D, owned by WP-7 (P15); WP-8 consumes it verbatim.
//! To avoid pre-empting that freeze, the subcommands here carry no flags yet;
//! later packets add them. Every subcommand currently dispatches to a stub that
//! reports "not yet implemented" and exits non-zero, so an unfinished path can
//! never masquerade as success.

use clap::{Parser, Subcommand, ValueEnum};

/// Top-level `rejolt` CLI parser.
#[derive(Debug, Parser)]
#[command(
    name = "rejolt",
    version,
    about = "Routed-memory reseed engine (WP-0 skeleton).",
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// The nine D20 subcommands plus the `hook` adapter entry mode. The parenthetical
/// on each variant names the work packet / plan item that fills it in.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Seed a clean, empty store (WP-7 / P14).
    Bootstrap,
    /// Rebuild the compiled routing artifacts from the store (WP-2 / P4).
    Rebuild,
    /// Validate the store and grammar (WP-1 / WP-7).
    Validate,
    /// Write-guard check for a full-file memory write (WP-4 / P9).
    CheckWrite,
    /// Collision projection for a proposed trigger set (WP-4 / P10).
    Project,
    /// Recall probe over a host event (WP-3 / P6).
    Search,
    /// Self-curation maintenance pass (WP-6 / P12).
    Maintain,
    /// Seat governance report / proposal (WP-6 / P12).
    Seats,
    /// Performance bench + calibration (WP-7 / P13).
    Bench,
    /// Host hook entry: `rejolt hook <event>` (WP-5 / P8).
    Hook {
        /// The host lifecycle event being dispatched (payload arrives on stdin).
        #[arg(value_enum)]
        event: HookEvent,
    },
}

impl Command {
    /// Stable lowercase name for diagnostics. Kept in one place so the stub
    /// message and any later logging agree.
    fn name(&self) -> &'static str {
        match self {
            Command::Bootstrap => "bootstrap",
            Command::Rebuild => "rebuild",
            Command::Validate => "validate",
            Command::CheckWrite => "check-write",
            Command::Project => "project",
            Command::Search => "search",
            Command::Maintain => "maintain",
            Command::Seats => "seats",
            Command::Bench => "bench",
            Command::Hook { .. } => "hook",
        }
    }
}

/// The frozen v1 hook event set (D19). Rendered by clap as the kebab-case
/// tokens `session-start` / `pre-op` / `post-op`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HookEvent {
    /// Session lifecycle start.
    SessionStart,
    /// Pre-operation (before a tool call commits).
    PreOp,
    /// Post-operation (after a tool call).
    PostOp,
}

/// Parse argv and dispatch. Returns the process exit code (see `main`).
pub fn run() -> i32 {
    let cli = Cli::parse();
    dispatch(&cli.command)
}

/// Dispatch a parsed command. WP-0 has no implemented subcommands; later
/// packets replace this per the WP map. The stub exits non-zero so that an
/// unimplemented path is never mistaken for a successful run. (The `hook`
/// mode's real exit taxonomy — never exit 1, per A5/D20 — lands with WP-5.)
pub fn dispatch(command: &Command) -> i32 {
    eprintln!(
        "rejolt: `{}` is not yet implemented (WP-0 skeleton)",
        command.name()
    );
    1
}
