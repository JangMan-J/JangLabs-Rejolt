//! CLI smoke tests: the clap parser resolves `--help`/`--version` without
//! panicking, and the binary's reported version matches the library const.
//! Drives the actual built binary via `CARGO_BIN_EXE_rejolt`.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rejolt")
}

#[test]
fn help_exits_zero_and_names_binary() {
    let output = Command::new(bin())
        .arg("--help")
        .output()
        .expect("run --help");
    assert!(output.status.success(), "--help should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("rejolt"),
        "help text should name the binary"
    );
    // The nine subcommands + hook mode should all be advertised.
    for sub in [
        "bootstrap",
        "rebuild",
        "validate",
        "check-write",
        "project",
        "search",
        "maintain",
        "seats",
        "bench",
        "hook",
    ] {
        assert!(stdout.contains(sub), "help should list `{sub}`");
    }
}

#[test]
fn version_exits_zero_and_matches_lib_const() {
    let output = Command::new(bin())
        .arg("--version")
        .output()
        .expect("run --version");
    assert!(output.status.success(), "--version should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(rejolt::VERSION),
        "binary --version ({stdout:?}) should contain lib VERSION ({})",
        rejolt::VERSION
    );
}
