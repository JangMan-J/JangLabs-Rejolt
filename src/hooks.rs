//! The `--print-hooks` settings block (plan P14, Appendix C; D13, N7, R6).
//!
//! The engine **never writes host permission policy or host settings** (D13/N7,
//! §0/§12). Bootstrap's `--print-hooks` EMITS the Claude Code `hooks` settings
//! block to STDOUT for the human to place into their user-global
//! `~/.claude/settings.json`; the engine writes nothing. This module builds that
//! block and nothing else — there is no filesystem write anywhere in it.
//!
//! ## Matcher strings carried verbatim (Appendix C / R6)
//!
//! The v1 matcher strings are the proven synapse wiring
//! (`settings.global.fragment.json`), carried verbatim — generalization is
//! post-v1 (orphan-ledger cut). Where synapse split the memory subsystem across
//! several `.sh` hooks, the reseed multiplexes one binary: `rejolt hook <event>`
//! dispatches recall / write-guard / write-context internally (P8), so one command
//! sits under each (event, matcher) block.
//!
//! - **SessionStart** (no matcher; re-fires on startup/resume/clear/compact) →
//!   `hook session-start`.
//! - **PreToolUse** over the native tool set `Bash|Read|Edit|Write|MultiEdit|
//!   WebFetch|WebSearch` AND the Context7 MCP matcher → `hook pre-op` (recall +,
//!   for Edit/Write/MultiEdit, the write-guard / write-context branches).
//! - **PostToolUse** over `Edit|Write|MultiEdit` and `Read` → `hook post-op`
//!   (read-signal + catalog refresh).

use serde_json::{Value, json};

/// The native-tool PreToolUse matcher (Appendix C, verbatim). Recall fires on any
/// of these; the write-guard / write-context branches act only on Edit/Write/
/// MultiEdit (decided by the internal dispatch, not the matcher).
pub const NATIVE_PRE_MATCHER: &str = "Bash|Read|Edit|Write|MultiEdit|WebFetch|WebSearch";
/// The Context7 MCP PreToolUse matcher (Appendix C, verbatim from the synapse
/// fragment) — so a context7 library lookup also triggers recall.
pub const CONTEXT7_PRE_MATCHER: &str = "mcp__plugin_context7_context7__.*";
/// The write-side PostToolUse matcher (catalog refresh on a store write).
pub const WRITE_POST_MATCHER: &str = "Edit|Write|MultiEdit";
/// The read-side PostToolUse matcher (the read-signal surface).
pub const READ_POST_MATCHER: &str = "Read";

/// The best-effort absolute path of the running `rejolt` binary, for the emitted
/// hook commands. Falls back to the bare name `rejolt` (on `$PATH`) if the exe
/// path cannot be resolved.
pub fn current_bin() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_else(|| "rejolt".to_string())
}

/// One `{ "type":"command", "command": "<bin> hook <event>", "timeout": N }` entry.
fn command_hook(bin: &str, event: &str, timeout: u64) -> Value {
    json!({
        "type": "command",
        "command": format!("{bin} hook {event}"),
        "timeout": timeout,
    })
}

/// Build the Claude Code `hooks` settings block wiring `rejolt hook <event>` under
/// the Appendix C matchers. `rejolt_bin` is the binary path the host will invoke.
/// This is a PURE builder — it writes nothing (D13/N7).
pub fn hooks_settings_block(rejolt_bin: &str) -> Value {
    json!({
        "_comment":
            "rejolt memory hooks — PLACE these under the `hooks` key of your user-global \
             ~/.claude/settings.json. The rejolt engine NEVER writes host settings or \
             permission policy (D13); this block is emitted for you to place. Matcher strings \
             are the proven v1 wiring, carried verbatim (Appendix C). `rejolt hook <event>` \
             multiplexes recall / write-guard / write-context internally.",
        "hooks": {
            "SessionStart": [
                { "hooks": [ command_hook(rejolt_bin, "session-start", 5) ] }
            ],
            "PreToolUse": [
                { "matcher": NATIVE_PRE_MATCHER, "hooks": [ command_hook(rejolt_bin, "pre-op", 10) ] },
                { "matcher": CONTEXT7_PRE_MATCHER, "hooks": [ command_hook(rejolt_bin, "pre-op", 5) ] }
            ],
            "PostToolUse": [
                { "matcher": WRITE_POST_MATCHER, "hooks": [ command_hook(rejolt_bin, "post-op", 10) ] },
                { "matcher": READ_POST_MATCHER, "hooks": [ command_hook(rejolt_bin, "post-op", 10) ] }
            ]
        }
    })
}

/// Render the settings block as pretty JSON (newline-terminated) for stdout.
pub fn render_print_hooks(rejolt_bin: &str) -> String {
    let mut s = serde_json::to_string_pretty(&hooks_settings_block(rejolt_bin))
        .expect("hooks settings block serializes (owned JSON only)");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_is_valid_json_and_names_the_binary() {
        let text = render_print_hooks("/usr/local/bin/rejolt");
        let parsed: Value = serde_json::from_str(&text).expect("emitted block is valid JSON");
        // The three host events are wired.
        let hooks = &parsed["hooks"];
        assert!(hooks["SessionStart"].is_array());
        assert!(hooks["PreToolUse"].is_array());
        assert!(hooks["PostToolUse"].is_array());
        // The verbatim matcher strings are present.
        assert_eq!(hooks["PreToolUse"][0]["matcher"], NATIVE_PRE_MATCHER);
        assert_eq!(hooks["PreToolUse"][1]["matcher"], CONTEXT7_PRE_MATCHER);
        // The command names the binary + event.
        assert_eq!(
            hooks["PreToolUse"][0]["hooks"][0]["command"],
            "/usr/local/bin/rejolt hook pre-op"
        );
        assert_eq!(
            hooks["SessionStart"][0]["hooks"][0]["command"],
            "/usr/local/bin/rejolt hook session-start"
        );
    }

    #[test]
    fn block_carries_no_permission_policy_keys() {
        // D13/N7: the emitted block wires HOOKS ONLY — never permissions / allow /
        // deny / defaultMode. (The synapse fragment's own _comment states the same.)
        let text = render_print_hooks("rejolt");
        for forbidden in [
            "permissions",
            "\"allow\"",
            "\"deny\"",
            "defaultMode",
            "bypass",
        ] {
            assert!(
                !text.contains(forbidden),
                "print-hooks must not emit permission-policy key `{forbidden}`"
            );
        }
    }
}
