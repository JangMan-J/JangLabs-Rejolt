//! Thin CLI entry point for the `rejolt` binary.
//!
//! All logic lives in the library crate (`rejolt::cli`); `main` only forwards
//! the process exit code so the binary and the library stay in lockstep and
//! integration tests can drive the same code path.

fn main() {
    std::process::exit(rejolt::cli::run());
}
