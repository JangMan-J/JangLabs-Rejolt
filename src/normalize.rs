//! Host-event parser → the typed [`NormalizedOp`] (plan P7 / WP-3; D19, A5, D15;
//! Appendix B is the freeze).
//!
//! This is the **adapter layer**: the one place anything Claude-specific lives.
//! It deserializes a Claude Code hook JSON payload (delivered on stdin as a
//! [`serde_json::Value`]) into the frozen internal [`NormalizedOp`]; the recall
//! engine ([`crate::recall`]) and — later — the write guard consume *only*
//! `NormalizedOp`, never a raw host payload (D19: "nothing Claude-specific below
//! the adapter layer").
//!
//! ## Fail-open posture (A5(b) — the contract)
//!
//! Every parse path fails **open, silently**: unknown JSON fields are ignored,
//! missing optionals are tolerated, and an unparseable / unclassifiable event is
//! never an error that could block a host operation — it becomes
//! [`NormalizedOp::Unclassifiable`], which recall treats as a no-op (silence).
//! Exit codes and hook dispatch are NOT decided here (that is WP-5); this module
//! only turns bytes into the typed op.
//!
//! ## The closed v1 tool set (Appendix B / A5(c))
//!
//! Guardable/extractable tools: `Bash`, `Read`, `Edit`, `Write`, `MultiEdit`,
//! `WebSearch`, `WebFetch`, and the proven MCP-context7 matcher
//! (`mcp__…context7…`). A tool outside the set still normalizes (fail-open): its
//! `ToolOp` simply carries no `command_text` / `target_path`, and recall extracts
//! whatever tokens it can (usually none). `MultiEdit` is kept in the frozen set
//! even though a current host build may fold multi-replace into `Edit`; the branch
//! is harmless if the host never emits it.
//!
//! ## `is_full_write` decided ONCE (Appendix B / A5(c) / D6)
//!
//! `is_full_write = Write` with non-empty `content`, decided a single time in the
//! normalizer. It is **tool-gated** (like `command_text`'s Bash gate), not keyed on
//! content presence alone: `Write` is the sole full-content tool → `true` +
//! `proposed_content` set; `Edit` / `MultiEdit` are partial → `false`, with **no
//! reconstruction** of the post-edit file (D6 parity). Gating on tool identity
//! means a crafted or schema-evolved Edit/MultiEdit payload that happens to carry a
//! `content` key can never flip a partial edit into a guardable full write — which
//! would let a later guard fail *closed* (N13) on an edit it must not judge.
//!
//! ## Canonicalization split (§5.x)
//!
//! This module supplies the **adapter-lexical** canonicalizer
//! ([`canonicalize_lexical`], `realpath -sm` semantics: absolutize against cwd +
//! resolve `.`/`..` lexically, symlinks NOT resolved). Recall feeds its byPath
//! query paths through it. Engine-realpath (symlink-resolving) canonicalization is
//! a placement concern for a later packet (WP-4), deliberately not done here.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

// =============================================================================
// The frozen NormalizedOp (Appendix B)
// =============================================================================

/// The internal normalized operation the engine consumes (Appendix B freeze).
///
/// The three host event kinds are `session-start` / `pre-op` / `post-op`;
/// [`NormalizedOp::Unclassifiable`] is the A5(b) typed fail-open sentinel for a
/// payload that could not be classified — it is not a host event kind, it is the
/// "no-op" recall reads as silence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NormalizedOp {
    /// A SessionStart event. Carries only the working directory.
    SessionStart {
        /// The session's working directory, if the payload supplied one.
        cwd: Option<PathBuf>,
    },
    /// A PreToolUse event — the advisory recall / write-guard surface.
    PreOp(ToolOp),
    /// A PostToolUse event — the read-signal / rebuild-refresh surface.
    PostOp(ToolOp),
    /// A payload that could not be classified (missing/unknown `hook_event_name`,
    /// non-object input, …). Fail-open (A5(b)): recall treats it as a no-op.
    Unclassifiable,
}

/// A normalized tool operation (Appendix B, fields EXACT). `raw_tool_input` is
/// carried verbatim so recall can pull tool-specific keys (WebSearch `query`,
/// WebFetch `url`, context7 library names) without a second host-shaped struct.
///
/// Note: no `Eq` — [`serde_json::Value`] is only `PartialEq` (it holds `f64`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ToolOp {
    /// The host tool name (`Bash`, `Read`, `Edit`, `Write`, `MultiEdit`,
    /// `WebSearch`, `WebFetch`, or an `mcp__…` name — or anything else, fail-open).
    pub tool_name: String,
    /// The raw `tool_input` object, verbatim (recall reads WebSearch/WebFetch/
    /// context7 keys from here).
    pub raw_tool_input: Value,
    /// The working directory, if supplied.
    pub cwd: Option<PathBuf>,
    /// The Bash command text — **Bash only**, `None` otherwise.
    pub command_text: Option<String>,
    /// The single target path (`file_path` // `path`) for path-bearing tools.
    pub target_path: Option<PathBuf>,
    /// Path-like tokens extracted from a Bash command (raw, not yet canonicalized).
    pub bash_embedded_paths: Vec<PathBuf>,
    /// The full proposed file content — set **only** when `is_full_write` (Write).
    pub proposed_content: Option<String>,
    /// Whether this is a full-file write (content present). Decided ONCE here.
    pub is_full_write: bool,
}

// =============================================================================
// The parser (host JSON → NormalizedOp) — fail open on every path (A5(b))
// =============================================================================

/// Parse a Claude Code hook payload into a [`NormalizedOp`]. Never errors, never
/// panics: an unrecognized / malformed payload yields
/// [`NormalizedOp::Unclassifiable`] (A5(b) fail-open). Unknown fields are ignored,
/// missing optionals tolerated.
pub fn parse_host_event(payload: &Value) -> NormalizedOp {
    let Some(event) = payload.get("hook_event_name").and_then(Value::as_str) else {
        // No event name (or a non-string one) — unclassifiable, fail open.
        return NormalizedOp::Unclassifiable;
    };
    let cwd = string_path(payload.get("cwd"));
    match event {
        "SessionStart" => NormalizedOp::SessionStart { cwd },
        "PreToolUse" => NormalizedOp::PreOp(normalize_tool_op(payload, cwd)),
        "PostToolUse" => NormalizedOp::PostOp(normalize_tool_op(payload, cwd)),
        // Any other event kind (UserPromptSubmit, Stop, …) is not a recall/guard
        // surface — fail open to a no-op.
        _ => NormalizedOp::Unclassifiable,
    }
}

/// Build a [`ToolOp`] from a pre/post-tool payload. Tolerates a missing
/// `tool_name` / `tool_input` (fail-open: an empty tool op that extracts nothing).
fn normalize_tool_op(payload: &Value, cwd: Option<PathBuf>) -> ToolOp {
    let tool_name = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let raw_tool_input = payload.get("tool_input").cloned().unwrap_or(Value::Null);
    let ti = &raw_tool_input;

    // command_text is Bash-only (Appendix B).
    let command_text = if tool_name == "Bash" {
        ti.get("command")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    } else {
        None
    };

    // target_path: `file_path`, falling back to `path` (the guarded/read tools).
    let target_path = string_path(ti.get("file_path")).or_else(|| string_path(ti.get("path")));

    // Embedded Bash paths come from the ONE tokenizer (parity with recall).
    let bash_embedded_paths = command_text
        .as_deref()
        .map(|c| tokenize_bash(c).embedded_paths)
        .unwrap_or_default();

    // is_full_write = `Write` with content, decided ONCE (A5(c)/D6). It is
    // **tool-gated**, not content-presence alone: `Write` is the sole full-content
    // tool; Edit/MultiEdit are partial with NO reconstruction. Gating on tool
    // identity (like `command_text`'s Bash gate) means a crafted or schema-evolved
    // Edit/MultiEdit payload that happens to carry a `content` key can never flip a
    // partial edit to a guardable full write — which would let WP-4 fail CLOSED
    // (N13) on an edit it must not guard. proposed_content is set iff full.
    let content = ti.get("content").and_then(Value::as_str);
    let is_full_write = tool_name == "Write" && content.map(|s| !s.is_empty()).unwrap_or(false);
    let proposed_content = if is_full_write {
        content.map(str::to_string)
    } else {
        None
    };

    ToolOp {
        tool_name,
        raw_tool_input,
        cwd,
        command_text,
        target_path,
        bash_embedded_paths,
        proposed_content,
        is_full_write,
    }
}

/// A non-empty JSON string as a [`PathBuf`], else `None` (missing optional
/// tolerated, empty string treated as absent).
fn string_path(v: Option<&Value>) -> Option<PathBuf> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

// =============================================================================
// Bash tokenizer (Appendix B parity rules) — the ONE tokenizer
// =============================================================================

/// Privilege / runner prefixes stripped per segment (Appendix B; synapse
/// `_PRIVILEGE` + the `env` runner).
const PRIVILEGE: &[&str] = &["sudo", "doas", "pkexec", "env"];

/// Runner value-flags: when a privilege runner is in play, these consume a
/// following value (`sudo -u bob …`). Synapse `_RUNNER_VALUE_FLAGS`.
const RUNNER_VALUE_FLAGS: &[&str] = &[
    "-u", "-g", "--user", "--group", "-p", "-C", "-r", "-t", "-h",
];

/// What [`tokenize_bash`] pulls from a Bash command text. The ONE tokenizer:
/// [`normalize_tool_op`] reads `embedded_paths` to fill `ToolOp`, and recall reads
/// `command_basenames` + `arg_tokens` — so the parse side and the recall side can
/// never disagree on how a command splits (Appendix B parity).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BashTokens {
    /// Command basenames (`words[0]` after prefix-stripping, basename via last `/`).
    pub command_basenames: Vec<String>,
    /// Path-like tokens (`/…` or `~/…`), raw (not yet canonicalized).
    pub embedded_paths: Vec<PathBuf>,
    /// Content-bearing argument tokens (non-flag, non-path words after the command).
    pub arg_tokens: Vec<String>,
}

/// Tokenize a Bash command text per the frozen parity rules (Appendix B):
///
/// 1. **Segment** on `;` / `&&` / `||` / `|` / newline (a lone `&` is NOT a
///    separator — parity with synapse's `(?:;|&&|\|\||\||\n)`).
/// 2. Per segment, **naive surrounding-quote strip** each word (`"`/`'` at either
///    end — a documented parity choice, not a real shell lexer).
/// 3. **Strip prefixes**: privilege runners (`sudo`/`doas`/`pkexec`/`env`),
///    `VAR=val` assignments, and — once a runner was seen — runner value-flags and
///    valueless runner flags.
/// 4. The first remaining word is the command; its **basename is via the last
///    `/`**. Non-flag, non-path words after it are content args; `/…` and `~/…`
///    words are embedded paths.
///
/// Deliberately NOT filtered by a generic-command stop-list here — the reseed
/// applies its GENERIC_VERBS stop-list at the recall gate (`grep` survives
/// tokenization as a basename), keeping tokenization a pure lexical pass.
pub fn tokenize_bash(command_text: &str) -> BashTokens {
    let mut out = BashTokens::default();
    for segment in split_segments(command_text) {
        let mut words: Vec<String> = segment
            .split_whitespace()
            .map(strip_surrounding_quotes)
            .filter(|w| !w.is_empty())
            .collect();
        strip_prefixes(&mut words);
        let Some(cmd_word) = words.first() else {
            continue;
        };
        let base = cmd_word.rsplit('/').next().unwrap_or(cmd_word).to_string();
        if !base.is_empty() {
            out.command_basenames.push(base);
        }
        for w in &words[1..] {
            if is_path_like(w) {
                out.embedded_paths.push(PathBuf::from(w));
            } else if !w.starts_with('-') {
                out.arg_tokens.push(w.clone());
            }
        }
        // The command word itself can be a path (`/usr/bin/foo`, `~/bin/x`).
        if is_path_like(cmd_word) {
            out.embedded_paths.push(PathBuf::from(cmd_word));
        }
    }
    out
}

/// Strip the leading privilege / assignment / runner-flag prefixes from a segment's
/// words in place, leaving the command word first.
fn strip_prefixes(words: &mut Vec<String>) {
    let mut saw_runner = false;
    while let Some(w) = words.first() {
        if PRIVILEGE.contains(&w.as_str()) {
            saw_runner = true;
            words.remove(0);
        } else if is_var_assignment(w) {
            // `VAR=val` (with or without a runner in front).
            words.remove(0);
        } else if saw_runner && RUNNER_VALUE_FLAGS.contains(&w.as_str()) {
            // A runner value-flag consumes its value too (`sudo -u bob`).
            words.remove(0);
            if !words.is_empty() {
                words.remove(0);
            }
        } else if saw_runner && w.starts_with('-') {
            // A valueless runner flag (`sudo -i`).
            words.remove(0);
        } else {
            break;
        }
    }
}

/// Split a command text into segments on `;` / `&&` / `||` / `|` / newline. A lone
/// `&` stays literal (not a separator), matching the frozen parity regex.
fn split_segments(cmd: &str) -> Vec<String> {
    let chars: Vec<char> = cmd.chars().collect();
    let mut segments = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            ';' | '\n' => {
                segments.push(std::mem::take(&mut cur));
                i += 1;
            }
            '|' => {
                segments.push(std::mem::take(&mut cur));
                i += if next == Some('|') { 2 } else { 1 };
            }
            '&' if next == Some('&') => {
                segments.push(std::mem::take(&mut cur));
                i += 2;
            }
            _ => {
                cur.push(c);
                i += 1;
            }
        }
    }
    segments.push(cur);
    segments
}

/// Naive surrounding-quote strip: remove any run of leading/trailing `"`/`'`
/// (a documented parity choice — synapse's `w.strip("\"'")`, NOT a real lexer).
fn strip_surrounding_quotes(w: &str) -> String {
    w.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// A `VAR=val` env-assignment prefix: `^[A-Za-z_][A-Za-z0-9_]*=…`.
fn is_var_assignment(w: &str) -> bool {
    let Some(eq) = w.find('=') else {
        return false;
    };
    if eq == 0 {
        return false;
    }
    let mut name = w[..eq].chars();
    let first = name.next().expect("eq > 0 so the name is non-empty");
    (first.is_ascii_alphabetic() || first == '_')
        && name.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A path-like token: absolute (`/…`) or home-anchored (`~` / `~/…`).
fn is_path_like(w: &str) -> bool {
    w.starts_with('/') || w == "~" || w.starts_with("~/")
}

// =============================================================================
// §5.x adapter-lexical canonicalization (realpath -sm semantics)
// =============================================================================

/// Canonicalize a query path the **adapter** way (§5.x): expand a leading `~`,
/// absolutize against `cwd` if relative, then resolve `.` / `..` and collapse
/// `//` **lexically** — symlinks are NOT resolved (`realpath -sm`). Pure: touches
/// no filesystem. This is the canonicalization recall feeds the byPath scan; the
/// engine-realpath (symlink-resolving) variant is a later-packet placement concern.
pub fn canonicalize_lexical(raw: &Path, cwd: Option<&Path>) -> PathBuf {
    let expanded = expand_home(raw);
    let absolute = if expanded.is_absolute() {
        expanded
    } else if let Some(cwd) = cwd {
        cwd.join(expanded)
    } else {
        // No cwd to anchor against: best-effort, leave relative (still normalized).
        expanded
    };
    lexical_normalize(&absolute)
}

/// Expand a leading `~` / `~/` against `$HOME`. `HOME` unset → the tilde is left
/// literal (deterministic, no panic). Only the anchor is expanded.
fn expand_home(raw: &Path) -> PathBuf {
    let s = raw.to_string_lossy();
    let Some(home) = std::env::var_os("HOME") else {
        return raw.to_path_buf();
    };
    if s == "~" {
        PathBuf::from(home)
    } else if let Some(rest) = s.strip_prefix("~/") {
        PathBuf::from(home).join(rest)
    } else {
        raw.to_path_buf()
    }
}

/// Resolve `.` / `..` and collapse `//` lexically (no symlink resolution, no
/// filesystem access). Matches `realpath -sm`: `..` pops a preceding normal
/// component but never climbs past an absolute root.
fn lexical_normalize(p: &Path) -> PathBuf {
    let is_abs = p.is_absolute();
    let mut stack: Vec<std::ffi::OsString> = Vec::new();
    for comp in p.components() {
        match comp {
            Component::Prefix(_) | Component::RootDir => {} // root handled by `is_abs`
            Component::CurDir => {}
            Component::ParentDir => {
                let pop = matches!(stack.last(), Some(last) if last != "..");
                if pop {
                    stack.pop();
                } else if !is_abs {
                    // A relative path may keep leading `..`; an absolute one cannot
                    // climb past root, so `..` there is simply dropped.
                    stack.push("..".into());
                }
            }
            Component::Normal(c) => stack.push(c.to_os_string()),
        }
    }
    let mut out = PathBuf::new();
    if is_abs {
        out.push("/");
    }
    for c in stack {
        out.push(c);
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- parse_host_event -------------------------------------------------

    #[test]
    fn parses_session_start_pre_and_post() {
        let ss = parse_host_event(&json!({"hook_event_name": "SessionStart", "cwd": "/work"}));
        assert_eq!(
            ss,
            NormalizedOp::SessionStart {
                cwd: Some(PathBuf::from("/work"))
            }
        );
        let pre = parse_host_event(&json!({
            "hook_event_name": "PreToolUse", "tool_name": "Read",
            "tool_input": {"file_path": "/etc/hosts"}, "cwd": "/work",
        }));
        match pre {
            NormalizedOp::PreOp(op) => {
                assert_eq!(op.tool_name, "Read");
                assert_eq!(op.target_path, Some(PathBuf::from("/etc/hosts")));
                assert!(!op.is_full_write);
            }
            other => panic!("expected PreOp, got {other:?}"),
        }
        let post = parse_host_event(&json!({
            "hook_event_name": "PostToolUse", "tool_name": "Bash",
            "tool_input": {"command": "ls"},
        }));
        assert!(matches!(post, NormalizedOp::PostOp(_)));
    }

    #[test]
    fn is_full_write_is_write_with_content_decided_once() {
        let full_write = |op: NormalizedOp| -> (bool, Option<String>) {
            let NormalizedOp::PreOp(o) = op else {
                panic!("expected PreOp")
            };
            (o.is_full_write, o.proposed_content)
        };

        // GOOD (full write): a Write with content → full + proposed_content set.
        let (is_full, proposed) = full_write(parse_host_event(&json!({
            "hook_event_name": "PreToolUse", "tool_name": "Write",
            "tool_input": {"file_path": "/s/m.md", "content": "full body"},
        })));
        assert!(is_full, "Write with content → full write");
        assert_eq!(proposed.as_deref(), Some("full body"));

        // BAD (partial): Edit / MultiEdit carry no `content` → partial, NO
        // reconstruction, NO proposed_content (D6 parity).
        for tool in ["Edit", "MultiEdit"] {
            let (is_full, proposed) = full_write(parse_host_event(&json!({
                "hook_event_name": "PreToolUse", "tool_name": tool,
                "tool_input": {"file_path": "/s/m.md", "old_string": "a", "new_string": "b"},
            })));
            assert!(!is_full, "{tool} is a partial edit → not a full write");
            assert!(proposed.is_none(), "{tool} carries no proposed content");
        }

        // FIX 3 lock: is_full_write is TOOL-gated. A crafted / schema-evolved
        // Edit/MultiEdit/Read payload that DOES carry a `content` key must still be
        // partial — a `content` field on a non-Write tool can never flip it to a
        // guardable full write (else WP-4 could fail CLOSED on an edit, N13/D6).
        for tool in ["Edit", "MultiEdit", "Read"] {
            let (is_full, proposed) = full_write(parse_host_event(&json!({
                "hook_event_name": "PreToolUse", "tool_name": tool,
                "tool_input": {"file_path": "/s/m.md", "content": "SMUGGLED full body"},
            })));
            assert!(
                !is_full,
                "{tool} with a crafted content key is still partial"
            );
            assert!(
                proposed.is_none(),
                "{tool} must not reconstruct proposed content"
            );
        }
    }

    #[test]
    fn fail_open_on_malformed_and_unknown() {
        // Missing event name → Unclassifiable (never an error).
        assert_eq!(
            parse_host_event(&json!({"tool_name": "Bash"})),
            NormalizedOp::Unclassifiable
        );
        // Non-object payload → Unclassifiable.
        assert_eq!(
            parse_host_event(&json!("nonsense")),
            NormalizedOp::Unclassifiable
        );
        assert_eq!(parse_host_event(&Value::Null), NormalizedOp::Unclassifiable);
        // Unknown event kind → Unclassifiable.
        assert_eq!(
            parse_host_event(&json!({"hook_event_name": "UserPromptSubmit"})),
            NormalizedOp::Unclassifiable
        );
        // Unknown tool still normalizes (fail-open), just extracts little.
        let unknown = parse_host_event(&json!({
            "hook_event_name": "PreToolUse", "tool_name": "SomeFutureTool",
            "tool_input": {"weird": 1}, "unknown_top_field": true,
        }));
        let NormalizedOp::PreOp(op) = unknown else {
            panic!("expected PreOp")
        };
        assert_eq!(op.tool_name, "SomeFutureTool");
        assert!(op.command_text.is_none() && op.target_path.is_none());
    }

    // ---- tokenize_bash ----------------------------------------------------

    #[test]
    fn bash_tokenizer_parity_privilege_env_var_and_pipe() {
        // The frozen parity example: privilege + env + VAR stripped, segmented on `|`.
        let t = tokenize_bash("sudo VAR=1 nvidia-smi -q | grep foo");
        assert_eq!(t.command_basenames, vec!["nvidia-smi", "grep"]);
        assert_eq!(t.arg_tokens, vec!["foo"]); // `-q` is a flag, `foo` is content
        assert!(t.embedded_paths.is_empty());

        // BAD-side contrast: a lone `&` is NOT a separator (parity), so only ONE
        // segment/basename here — proving the segmenter does not over-split.
        let bg = tokenize_bash("myserver & echo done");
        assert_eq!(bg.command_basenames, vec!["myserver"]);
    }

    #[test]
    fn bash_tokenizer_basename_paths_and_quotes() {
        let t = tokenize_bash("cat \"/etc/foo.conf\" && /usr/bin/systemctl restart x");
        // basenames via last '/': cat, systemctl.
        assert_eq!(t.command_basenames, vec!["cat", "systemctl"]);
        // embedded paths: the quoted /etc/foo.conf and the /usr/bin/systemctl command.
        assert!(t.embedded_paths.contains(&PathBuf::from("/etc/foo.conf")));
        assert!(
            t.embedded_paths
                .contains(&PathBuf::from("/usr/bin/systemctl"))
        );
        // content args: restart, x (paths and flags excluded).
        assert_eq!(t.arg_tokens, vec!["restart", "x"]);
    }

    #[test]
    fn bash_segments_on_all_operators_and_newline() {
        let t = tokenize_bash("a; b && c || d | e\nf");
        assert_eq!(t.command_basenames, vec!["a", "b", "c", "d", "e", "f"]);
    }

    // ---- canonicalize_lexical (§5.x) --------------------------------------

    #[test]
    fn lexical_canonicalization_resolves_dot_dot_without_symlinks() {
        // GOOD: absolute, dot/dot-dot collapsed.
        assert_eq!(
            canonicalize_lexical(Path::new("/etc/foo/../bar/./baz"), None),
            PathBuf::from("/etc/bar/baz")
        );
        // Relative absolutized against cwd, then normalized.
        assert_eq!(
            canonicalize_lexical(Path::new("sub/../x"), Some(Path::new("/home/u"))),
            PathBuf::from("/home/u/x")
        );
        // `..` cannot climb past an absolute root.
        assert_eq!(
            canonicalize_lexical(Path::new("/../../x"), None),
            PathBuf::from("/x")
        );
        // Repeated slashes collapse.
        assert_eq!(
            canonicalize_lexical(Path::new("/a//b///c"), None),
            PathBuf::from("/a/b/c")
        );
    }

    #[test]
    fn lexical_canonicalization_expands_home_anchor() {
        // Read the real HOME rather than mutating it — a `set_var` here would race
        // parallel tests that read HOME (index/telemetry) under the test harness.
        let Some(home) = std::env::var_os("HOME") else {
            return; // no HOME on this host: the anchor stays literal, nothing to assert
        };
        let home = PathBuf::from(home);
        assert_eq!(
            canonicalize_lexical(Path::new("~/.config/nvim"), None),
            home.join(".config/nvim")
        );
        assert_eq!(canonicalize_lexical(Path::new("~"), None), home);
    }

    #[test]
    fn normalizedop_serde_round_trips() {
        // Appendix B: NormalizedOp is a serde enum — it round-trips through JSON.
        let op = NormalizedOp::PreOp(ToolOp {
            tool_name: "Bash".into(),
            raw_tool_input: json!({"command": "ls"}),
            cwd: Some(PathBuf::from("/w")),
            command_text: Some("ls".into()),
            target_path: None,
            bash_embedded_paths: vec![],
            proposed_content: None,
            is_full_write: false,
        });
        let json = serde_json::to_string(&op).unwrap();
        let back: NormalizedOp = serde_json::from_str(&json).unwrap();
        assert_eq!(op, back);
    }
}
