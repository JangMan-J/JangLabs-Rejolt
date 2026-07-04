//! The one bespoke parser in the system: the memory-frontmatter dialect
//! (D21, A3).
//!
//! Memory files are markdown with a `---`-fenced frontmatter block. That block
//! is a **formally constrained subset of YAML**, hand-rolled here rather than
//! delegated to a full-YAML crate. The rationale (A3, re-verified at build):
//! full-YAML semantics at the system's sole fail-closed boundary are a
//! liability, and the Rust serde-YAML crate landscape is poor (`serde_yaml`
//! archived 2024; successor forks unproven). This parser sits at the #1-rule
//! fail-closed surface — a **false deny here is the worst failure mode (RB3)** —
//! so the accepted and rejected shapes are frozen in plan Appendix B2 and
//! implemented exactly, never invented.
//!
//! ## The dialect boundary (Appendix B2)
//!
//! **In-subset (parsed):** `---` fences; top-level `name`/`description` as
//! single-line plain or single/double-quoted scalars; a `metadata:` map at
//! 2-space indentation carrying `tags`, optional `triggers` (exactly the four
//! arrays `commands`/`paths`/`args`/`synonyms`), and ranking fields; sequences
//! in both flow (`[a, b]`) and block (`- x`) form; full-line `#` comments;
//! UTF-8.
//!
//! **Out-of-subset (rejected, each citing the violated rule):** anchors/aliases
//! (`&`/`*`), type tags (`!!`), multi-document markers, block scalars (`|`/`>`),
//! flow mappings (`{}`), multiline strings, tab indentation, duplicate keys,
//! **top-level `triggers:`** (its own named error), unknown top-level or
//! metadata keys.
//!
//! ## The three-oracle discipline (A3/B2)
//!
//! Realized in `tests/frontmatter.rs`: (1) differential agreement with pinned
//! PyYAML `safe_load` over the in-subset corpus, run out-of-process in the test
//! harness only; (2) a committed expected-value vector corpus (the portable
//! fallback where Python is absent); (3) generate→parse→regenerate round-trip on
//! every in-subset vector. [`canonical_json`] and [`frontmatter_block`] exist to
//! feed the differential oracle; [`generate`] feeds the round-trip.

use std::fmt;

use crate::tag::is_tag;

// =============================================================================
// Public parsed model
// =============================================================================

/// A parsed memory frontmatter block.
///
/// `name` and `description` are optional: D21/B2 mark only `metadata.tags` as
/// required. The write guard's shape rule (§6) is "frontmatter present and a
/// valid `metadata` block"; it does not require name/description, so requiring
/// them here would be inventing a reject case at a fail-closed boundary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frontmatter {
    /// Top-level `name` scalar, if present.
    pub name: Option<String>,
    /// Top-level `description` scalar, if present.
    pub description: Option<String>,
    /// The required `metadata:` block.
    pub metadata: Metadata,
}

/// The `metadata:` block. `tags` is required (≥1 kebab-case entry); everything
/// else is optional.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Metadata {
    /// Required, ≥1 entry, each kebab-case (`TAG_RE`, see [`crate::tag`]).
    pub tags: Vec<String>,
    /// Optional behavioral evidence block. `None` iff no `triggers:` key was
    /// present; a present-but-partial block defaults its absent arrays to empty.
    pub triggers: Option<Triggers>,
    /// Ranking field `lastReviewed` (opaque scalar; consumed by curation).
    pub last_reviewed: Option<String>,
    /// Ranking field `declineCount` (integer; consumed by recall penalties).
    pub decline_count: Option<i64>,
}

/// The four trigger arrays. Absent arrays default to empty; presence of the
/// containing `triggers:` key is tracked by [`Metadata::triggers`] being `Some`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Triggers {
    /// `commands` — strong tier.
    pub commands: Vec<String>,
    /// `paths` — strong tier.
    pub paths: Vec<String>,
    /// `args` — medium tier.
    pub args: Vec<String>,
    /// `synonyms` — weak tier.
    pub synonyms: Vec<String>,
}

// =============================================================================
// Typed rejection reasons — each variant names a B2 rule (RB3)
// =============================================================================

/// A rejection reason. Each dialect variant names the Appendix B2 rule it
/// violated so a deny can cite the rule (RB3); schema variants name the D21
/// schema requirement broken. `line` is the 1-based file line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrontmatterError {
    // ---- fence / structure ----
    /// The file does not open with a `---` fence on line 1.
    MissingOpeningFence,
    /// No closing `---` fence was found after the opening fence.
    MissingClosingFence,

    // ---- B2 out-of-subset dialect rules ----
    /// An anchor (`&name`) in node position. Out of subset.
    Anchor { line: usize },
    /// An alias (`*name`) in node position — also how an unquoted `*.md` glob
    /// reads to YAML. Out of subset; quote the value.
    Alias { line: usize },
    /// A type tag (`!` / `!!type`). Out of subset.
    TypeTag { line: usize },
    /// A multi-document marker (`---` or `...`) inside the frontmatter block.
    MultiDocument { line: usize },
    /// A block scalar indicator (`|` / `>`). Out of subset.
    BlockScalar { line: usize },
    /// A flow mapping (`{ … }`) in node position (a value/item that STARTS with
    /// `{`). Out of subset; quote the value. Mid-scalar braces (`a{b}`,
    /// `~/.config/{nvim,vim}/**`) are literal and accepted, matching PyYAML.
    FlowMapping { line: usize },
    /// An inline `#` comment (a `#` at the start of, or preceded by whitespace
    /// within, a plain scalar or flow item). Only full-line `#` comments are
    /// in-subset (B2); a `#` NOT preceded by whitespace (`c#`, `/foo#bar`) is a
    /// literal and is kept. Out of subset.
    InlineComment { line: usize },
    /// A plain (unquoted) scalar containing `: ` (colon-space) — a mapping
    /// indicator, not plain-scalar content (PyYAML raises on it). Quote the
    /// value to carry a literal `: `. Out of subset.
    ColonInScalar { line: usize },
    /// A double-quoted escape other than `\"` `\\` `\n` `\t` `\r` (the escapes
    /// [`generate`] emits and can round-trip). `\xNN`/`\uNNNN`/etc. are out of
    /// subset rather than silently mangled.
    InvalidEscape { line: usize },
    /// A string that does not terminate on its line (unclosed quote, or a plain
    /// scalar continued on a following line). Out of subset.
    MultilineString { line: usize },
    /// A tab used for indentation. Out of subset (spaces only).
    TabIndentation { line: usize },
    /// The same mapping key appeared twice at one level.
    DuplicateKey { key: String, line: usize },
    /// `triggers:` at the document root. Its OWN named error (D21): triggers
    /// live only at `metadata.triggers`, never top level.
    TopLevelTriggers { line: usize },
    /// A top-level key other than `name` / `description` / `metadata`
    /// (top-level `tags:` also lands here, per D21 parity).
    UnknownTopLevelKey { key: String, line: usize },
    /// A `metadata` key other than the closed set
    /// {`tags`, `triggers`, `lastReviewed`, `declineCount`}.
    UnknownMetadataKey { key: String, line: usize },
    /// A `metadata.triggers` key other than
    /// {`commands`, `paths`, `args`, `synonyms`}.
    UnknownTriggerKey { key: String, line: usize },

    // ---- structural / schema (D21) ----
    /// A line that is not a comment, blank, fence, `key: value`, or `- item`.
    MalformedLine { line: usize },
    /// Indentation that does not correspond to any open block.
    BadIndentation { line: usize },
    /// A key expected to carry a scalar carried a sequence/mapping instead.
    ExpectedScalar { key: String, line: usize },
    /// A key expected to carry a sequence carried a scalar/mapping instead.
    ExpectedSequence { key: String, line: usize },
    /// A key expected to carry a nested mapping carried a scalar/sequence.
    ExpectedMapping { key: String, line: usize },
    /// `metadata.tags` is absent (required, D21).
    MissingTags,
    /// `metadata.tags` is present but empty (must have ≥1 entry, D21).
    EmptyTags,
    /// A tag is not kebab-case (`TAG_RE`, D21).
    InvalidTag { tag: String },
    /// A ranking field carried a value of the wrong shape (e.g. non-integer
    /// `declineCount`).
    InvalidRankingField { key: String, line: usize },
}

impl fmt::Display for FrontmatterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use FrontmatterError::*;
        match self {
            MissingOpeningFence => write!(f, "frontmatter must open with a `---` fence on line 1"),
            MissingClosingFence => write!(f, "frontmatter has no closing `---` fence"),
            Anchor { line } => write!(f, "line {line}: anchors (`&name`) are out of subset"),
            Alias { line } => write!(
                f,
                "line {line}: aliases (`*name`) are out of subset; quote glob values like `*.md`"
            ),
            TypeTag { line } => {
                write!(f, "line {line}: type tags (`!`/`!!type`) are out of subset")
            }
            MultiDocument { line } => write!(
                f,
                "line {line}: multi-document markers (`---`/`...`) are out of subset in frontmatter"
            ),
            BlockScalar { line } => {
                write!(f, "line {line}: block scalars (`|`/`>`) are out of subset")
            }
            FlowMapping { line } => write!(
                f,
                "line {line}: flow mappings (`{{ … }}`) are out of subset; quote the value"
            ),
            InlineComment { line } => write!(
                f,
                "line {line}: inline `#` comments are out of subset (only full-line comments); quote a literal ` #`"
            ),
            ColonInScalar { line } => write!(
                f,
                "line {line}: a plain scalar may not contain `: ` (colon-space); quote the value"
            ),
            InvalidEscape { line } => write!(
                f,
                "line {line}: unknown double-quote escape (allowed: \\\" \\\\ \\n \\t \\r)"
            ),
            MultilineString { line } => {
                write!(
                    f,
                    "line {line}: multiline / unterminated strings are out of subset"
                )
            }
            TabIndentation { line } => {
                write!(
                    f,
                    "line {line}: tab indentation is out of subset (use spaces)"
                )
            }
            DuplicateKey { key, line } => write!(f, "line {line}: duplicate key `{key}`"),
            TopLevelTriggers { line } => write!(
                f,
                "line {line}: `triggers:` is not allowed at the document root (use `metadata.triggers`)"
            ),
            UnknownTopLevelKey { key, line } => write!(
                f,
                "line {line}: unknown top-level key `{key}` (allowed: name, description, metadata)"
            ),
            UnknownMetadataKey { key, line } => write!(
                f,
                "line {line}: unknown metadata key `{key}` (allowed: tags, triggers, lastReviewed, declineCount)"
            ),
            UnknownTriggerKey { key, line } => write!(
                f,
                "line {line}: unknown triggers key `{key}` (allowed: commands, paths, args, synonyms)"
            ),
            MalformedLine { line } => {
                write!(f, "line {line}: not a valid key/value or sequence line")
            }
            BadIndentation { line } => write!(f, "line {line}: unexpected indentation"),
            ExpectedScalar { key, line } => write!(f, "line {line}: `{key}` must be a scalar"),
            ExpectedSequence { key, line } => write!(f, "line {line}: `{key}` must be a sequence"),
            ExpectedMapping { key, line } => write!(f, "line {line}: `{key}` must be a mapping"),
            MissingTags => write!(f, "metadata.tags is required"),
            EmptyTags => write!(f, "metadata.tags must have at least one entry"),
            InvalidTag { tag } => write!(
                f,
                "tag `{tag}` is not kebab-case ({})",
                crate::tag::TAG_PATTERN
            ),
            InvalidRankingField { key, line } => {
                write!(f, "line {line}: ranking field `{key}` has an invalid value")
            }
        }
    }
}

impl std::error::Error for FrontmatterError {}

// =============================================================================
// Internal generic tree — the "what YAML sees" layer
// =============================================================================
//
// The parser builds this constrained tree (dialect checks happen here), and two
// consumers read it: `interpret` applies the D21 schema (key whitelists, tags
// required, kebab), and `node_to_json` renders the differential-oracle view.
// Splitting the layers is what lets the differential compare exactly what YAML
// structurally sees, while the typed `Frontmatter` carries the schema.

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Scalar(String),
    Seq(Vec<String>),
    Map(Vec<MapEntry>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MapEntry {
    key: String,
    lineno: usize,
    value: Node,
}

/// A frontmatter content line, after indentation and comment/blank stripping.
struct Line {
    indent: usize,
    content: String,
    lineno: usize,
}

// =============================================================================
// Public API
// =============================================================================

/// Parse a full memory file (frontmatter + body) into a typed [`Frontmatter`],
/// applying the D21 schema. Only the frontmatter block is parsed; the body after
/// the closing fence is ignored.
pub fn parse(input: &str) -> Result<Frontmatter, FrontmatterError> {
    let lines = tokenize(input)?;
    let tree = parse_document(&lines)?;
    interpret(tree)
}

/// Serialize a [`Frontmatter`] back to canonical dialect text (round-trip
/// oracle). The output re-parses to an equal `Frontmatter`, and regenerating
/// from that is byte-identical.
pub fn generate(fm: &Frontmatter) -> String {
    let mut out = String::from("---\n");
    if let Some(name) = &fm.name {
        out.push_str("name: ");
        out.push_str(&emit_scalar(name));
        out.push('\n');
    }
    if let Some(desc) = &fm.description {
        out.push_str("description: ");
        out.push_str(&emit_scalar(desc));
        out.push('\n');
    }
    out.push_str("metadata:\n");
    out.push_str("  tags: ");
    out.push_str(&emit_flow_seq(&fm.metadata.tags));
    out.push('\n');
    if let Some(tr) = &fm.metadata.triggers {
        out.push_str("  triggers:\n");
        for (key, arr) in [
            ("commands", &tr.commands),
            ("paths", &tr.paths),
            ("args", &tr.args),
            ("synonyms", &tr.synonyms),
        ] {
            out.push_str("    ");
            out.push_str(key);
            out.push_str(": ");
            out.push_str(&emit_flow_seq(arr));
            out.push('\n');
        }
    }
    if let Some(lr) = &fm.metadata.last_reviewed {
        out.push_str("  lastReviewed: ");
        out.push_str(&emit_scalar(lr));
        out.push('\n');
    }
    if let Some(dc) = &fm.metadata.decline_count {
        out.push_str("  declineCount: ");
        out.push_str(&dc.to_string());
        out.push('\n');
    }
    out.push_str("---\n");
    out
}

/// Return the raw text between the frontmatter fences (exclusive), for feeding
/// the out-of-process PyYAML differential oracle the same bytes this parser
/// sees. Does not run dialect checks beyond locating the fences.
pub fn frontmatter_block(input: &str) -> Result<String, FrontmatterError> {
    let stripped: Vec<&str> = input
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    if stripped.first().map(|l| *l != "---").unwrap_or(true) {
        return Err(FrontmatterError::MissingOpeningFence);
    }
    let close = stripped
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, l)| **l == "---")
        .map(|(i, _)| i)
        .ok_or(FrontmatterError::MissingClosingFence)?;
    Ok(stripped[1..close].join("\n"))
}

/// Render the frontmatter's structural view as canonical (sorted-key) JSON, with
/// every scalar stringified. This is the Rust side of the differential oracle:
/// PyYAML `safe_load` of the same block, with scalars likewise stringified, must
/// produce an equal JSON value. Stringifying both sides sidesteps YAML implicit
/// typing (numbers/dates/bools as bare words), which is deliberately outside the
/// differential's in-subset corpus.
pub fn canonical_json(input: &str) -> Result<String, FrontmatterError> {
    let lines = tokenize(input)?;
    let tree = parse_document(&lines)?;
    let value = node_to_json(&tree);
    Ok(serde_json::to_string(&value).expect("serde_json cannot fail on string/array/object"))
}

// =============================================================================
// Tokenize: fences, comments, blanks, tab-indentation, multi-doc markers
// =============================================================================

fn tokenize(input: &str) -> Result<Vec<Line>, FrontmatterError> {
    let stripped: Vec<&str> = input
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();

    if stripped.first().map(|l| *l != "---").unwrap_or(true) {
        return Err(FrontmatterError::MissingOpeningFence);
    }
    let close = stripped
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, l)| **l == "---")
        .map(|(i, _)| i)
        .ok_or(FrontmatterError::MissingClosingFence)?;

    let mut lines = Vec::new();
    for (idx, raw) in stripped.iter().enumerate().take(close).skip(1) {
        let lineno = idx + 1;

        // A `---`/`...` inside the block is a document marker: multi-document.
        let trimmed = raw.trim();
        if trimmed == "---" || trimmed == "..." {
            return Err(FrontmatterError::MultiDocument { line: lineno });
        }

        // Measure indentation; a tab in the indent region is out of subset.
        let bytes = raw.as_bytes();
        let mut indent = 0usize;
        let mut j = 0usize;
        while j < bytes.len() {
            match bytes[j] {
                b' ' => {
                    indent += 1;
                    j += 1;
                }
                b'\t' => return Err(FrontmatterError::TabIndentation { line: lineno }),
                _ => break,
            }
        }
        // `j` only advanced over ASCII spaces, so this is a char boundary.
        let rest = raw[j..].trim_end();
        if rest.is_empty() {
            continue; // blank line
        }
        if rest.starts_with('#') {
            continue; // full-line comment
        }
        lines.push(Line {
            indent,
            content: rest.to_string(),
            lineno,
        });
    }
    Ok(lines)
}

// =============================================================================
// Structural parse: indentation-driven mapping / sequence tree
// =============================================================================

fn parse_document(lines: &[Line]) -> Result<Node, FrontmatterError> {
    if lines.is_empty() {
        return Ok(Node::Map(Vec::new()));
    }
    if lines[0].indent != 0 {
        return Err(FrontmatterError::BadIndentation {
            line: lines[0].lineno,
        });
    }
    let mut i = 0usize;
    let node = parse_block(lines, &mut i, 0)?;
    if i != lines.len() {
        return Err(FrontmatterError::BadIndentation {
            line: lines[i].lineno,
        });
    }
    Ok(node)
}

fn parse_block(lines: &[Line], i: &mut usize, indent: usize) -> Result<Node, FrontmatterError> {
    let first = &lines[*i];
    if is_seq_item(&first.content) {
        parse_seq(lines, i, indent)
    } else {
        parse_map(lines, i, indent)
    }
}

fn is_seq_item(content: &str) -> bool {
    content == "-" || content.starts_with("- ")
}

fn parse_seq(lines: &[Line], i: &mut usize, indent: usize) -> Result<Node, FrontmatterError> {
    let mut items = Vec::new();
    while *i < lines.len() {
        let line = &lines[*i];
        if line.indent != indent || !is_seq_item(&line.content) {
            break;
        }
        let item = if line.content == "-" {
            "" // empty item -> rejected below
        } else {
            line.content[2..].trim()
        };
        if item.is_empty() {
            return Err(FrontmatterError::MalformedLine { line: line.lineno });
        }
        items.push(parse_scalar_token(item, line.lineno)?);
        *i += 1;
    }
    Ok(Node::Seq(items))
}

fn parse_map(lines: &[Line], i: &mut usize, indent: usize) -> Result<Node, FrontmatterError> {
    let mut entries: Vec<MapEntry> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    while *i < lines.len() {
        let line = &lines[*i];
        if line.indent < indent {
            break; // belongs to an outer block
        }
        if line.indent > indent {
            return Err(FrontmatterError::BadIndentation { line: line.lineno });
        }
        if is_seq_item(&line.content) {
            // A sequence item where a mapping key was expected.
            return Err(FrontmatterError::MalformedLine { line: line.lineno });
        }

        let colon = find_key_colon(&line.content)
            .ok_or(FrontmatterError::MultilineString { line: line.lineno })?;
        let key = line.content[..colon].trim().to_string();
        if key.is_empty() {
            return Err(FrontmatterError::MalformedLine { line: line.lineno });
        }
        if seen.contains(&key) {
            return Err(FrontmatterError::DuplicateKey {
                key,
                line: line.lineno,
            });
        }
        seen.push(key.clone());
        let lineno = line.lineno;
        let value_str = line.content[colon + 1..].trim().to_string();
        *i += 1;

        let value = if value_str.is_empty() {
            parse_nested(lines, i, indent)?
        } else {
            parse_inline_value(&value_str, lineno)?
        };
        entries.push(MapEntry { key, lineno, value });
    }
    Ok(Node::Map(entries))
}

/// After an empty `key:` value, the value is a deeper mapping, a block sequence
/// (either compact at the key's indent or deeper), or an empty scalar.
fn parse_nested(lines: &[Line], i: &mut usize, indent: usize) -> Result<Node, FrontmatterError> {
    if *i >= lines.len() {
        return Ok(Node::Scalar(String::new()));
    }
    let next = &lines[*i];
    if next.indent > indent {
        let child = next.indent;
        parse_block(lines, i, child)
    } else if next.indent == indent && is_seq_item(&next.content) {
        parse_seq(lines, i, indent)
    } else {
        Ok(Node::Scalar(String::new()))
    }
}

/// The first `:` that terminates a mapping key: one followed by a space or at
/// end of line. `key:value` (no space) is therefore not a key line, matching
/// YAML.
fn find_key_colon(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    for (idx, &c) in bytes.iter().enumerate() {
        if c == b':' && (idx + 1 == bytes.len() || bytes[idx + 1] == b' ') {
            return Some(idx);
        }
    }
    None
}

// =============================================================================
// Scalar / flow-sequence value parsing (dialect gate lives here)
// =============================================================================

fn parse_inline_value(s: &str, lineno: usize) -> Result<Node, FrontmatterError> {
    let first = s.chars().next().expect("value is non-empty after trim");
    match first {
        '[' => parse_flow_seq(s, lineno),
        '{' => Err(FrontmatterError::FlowMapping { line: lineno }),
        _ => Ok(Node::Scalar(parse_scalar_token(s, lineno)?)),
    }
}

/// Parse one scalar token (a mapping value or a sequence item), running the B2
/// dialect gate and un-quoting. Returns the string value.
fn parse_scalar_token(tok: &str, lineno: usize) -> Result<String, FrontmatterError> {
    let first = tok.chars().next().expect("token is non-empty after trim");
    match first {
        '&' => return Err(FrontmatterError::Anchor { line: lineno }),
        '*' => return Err(FrontmatterError::Alias { line: lineno }),
        '!' => return Err(FrontmatterError::TypeTag { line: lineno }),
        '{' | '}' => return Err(FrontmatterError::FlowMapping { line: lineno }),
        '[' => return Err(FrontmatterError::MalformedLine { line: lineno }), // nested flow seq
        '|' | '>' => return Err(FrontmatterError::BlockScalar { line: lineno }),
        '\'' => return parse_single_quoted(tok, lineno),
        '"' => return parse_double_quoted(tok, lineno),
        _ => {}
    }
    // Plain scalar. A genuine flow mapping is a value that STARTS with `{`
    // (caught above); mid-scalar braces (`a{b}`) are literal, matching PyYAML,
    // so there is deliberately no blanket brace guard here.
    //
    // Two plain-scalar rules mirror YAML / PyYAML exactly (see B2):
    //  - a `#` at the start of, or preceded by whitespace within, the token
    //    begins an inline comment (only full-line comments are in-subset). A `#`
    //    NOT preceded by whitespace (`c#`, `/foo#bar`) is a literal, kept.
    //  - `: ` (colon-space) is a mapping indicator, not plain content.
    if plain_has_inline_comment(tok) {
        return Err(FrontmatterError::InlineComment { line: lineno });
    }
    if tok.contains(": ") {
        return Err(FrontmatterError::ColonInScalar { line: lineno });
    }
    Ok(tok.to_string())
}

/// Whether a plain (already-trimmed) scalar token contains an inline `#`
/// comment: a `#` at position 0 (which, in every context this token is formed —
/// a mapping value after `: `, a block-seq item after `- `, or a flow item —
/// was whitespace-preceded before trimming) or a `#` preceded by a space/tab.
fn plain_has_inline_comment(tok: &str) -> bool {
    let bytes = tok.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        if c == b'#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return true;
        }
    }
    false
}

fn parse_flow_seq(s: &str, lineno: usize) -> Result<Node, FrontmatterError> {
    if !s.ends_with(']') {
        return Err(FrontmatterError::MultilineString { line: lineno }); // unterminated flow
    }
    let inner = &s[1..s.len() - 1];
    let mut out = Vec::new();
    for raw in split_flow_items(inner, lineno)? {
        let item = raw.trim();
        if item.is_empty() {
            continue; // tolerate a trailing comma
        }
        out.push(parse_scalar_token(item, lineno)?);
    }
    Ok(Node::Seq(out))
}

/// Split a flow-sequence body on top-level commas, respecting quotes.
fn split_flow_items(inner: &str, lineno: usize) -> Result<Vec<String>, FrontmatterError> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut chars = inner.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => {
                cur.push(c);
                if in_single {
                    if chars.peek() == Some(&'\'') {
                        cur.push('\'');
                        chars.next();
                    } else {
                        in_single = false;
                    }
                } else {
                    in_single = true;
                }
            }
            '"' if !in_single => {
                cur.push(c);
                in_double = !in_double;
            }
            ',' if !in_single && !in_double => {
                items.push(std::mem::take(&mut cur));
            }
            // In flow context (unlike block plain scalars) a `{`/`}` is a
            // flow-mapping indicator, not a literal — PyYAML raises on `[a{b}]`.
            // Quote the value to carry a literal brace inside a flow sequence.
            '{' | '}' if !in_single && !in_double => {
                return Err(FrontmatterError::FlowMapping { line: lineno });
            }
            _ => cur.push(c),
        }
    }
    if in_single || in_double {
        return Err(FrontmatterError::MultilineString { line: lineno });
    }
    items.push(cur);
    Ok(items)
}

fn parse_single_quoted(tok: &str, lineno: usize) -> Result<String, FrontmatterError> {
    let mut it = tok.chars();
    it.next(); // opening '
    let mut out = String::new();
    loop {
        match it.next() {
            None => return Err(FrontmatterError::MultilineString { line: lineno }),
            Some('\'') => {
                // Peek without consuming by cloning the cheap Chars iterator.
                let mut look = it.clone();
                if look.next() == Some('\'') {
                    out.push('\''); // '' escape
                    it.next();
                } else if it.next().is_some() {
                    return Err(FrontmatterError::MalformedLine { line: lineno }); // trailing junk
                } else {
                    return Ok(out);
                }
            }
            Some(c) => out.push(c),
        }
    }
}

fn parse_double_quoted(tok: &str, lineno: usize) -> Result<String, FrontmatterError> {
    let mut it = tok.chars();
    it.next(); // opening "
    let mut out = String::new();
    loop {
        match it.next() {
            None => return Err(FrontmatterError::MultilineString { line: lineno }),
            Some('\\') => match it.next() {
                None => return Err(FrontmatterError::MultilineString { line: lineno }),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                // Only the escapes `generate` emits are in-subset; anything else
                // (`\xNN`, `\uNNNN`, `\0`, …) is rejected rather than silently
                // mangled into a divergence from PyYAML.
                Some(_) => return Err(FrontmatterError::InvalidEscape { line: lineno }),
            },
            Some('"') => {
                if it.next().is_some() {
                    return Err(FrontmatterError::MalformedLine { line: lineno }); // trailing junk
                }
                return Ok(out);
            }
            Some(c) => out.push(c),
        }
    }
}

// =============================================================================
// Interpret: apply the D21 schema over the tree
// =============================================================================

fn interpret(tree: Node) -> Result<Frontmatter, FrontmatterError> {
    let entries = match tree {
        Node::Map(e) => e,
        _ => return Err(FrontmatterError::MalformedLine { line: 2 }),
    };
    let mut fm = Frontmatter::default();
    let mut have_metadata = false;
    for MapEntry { key, lineno, value } in entries {
        match key.as_str() {
            "name" => fm.name = Some(expect_scalar(value, "name", lineno)?),
            "description" => fm.description = Some(expect_scalar(value, "description", lineno)?),
            "metadata" => {
                have_metadata = true;
                fm.metadata = interpret_metadata(value, lineno)?;
            }
            "triggers" => return Err(FrontmatterError::TopLevelTriggers { line: lineno }),
            other => {
                return Err(FrontmatterError::UnknownTopLevelKey {
                    key: other.to_string(),
                    line: lineno,
                });
            }
        }
    }
    if !have_metadata {
        return Err(FrontmatterError::MissingTags);
    }
    Ok(fm)
}

fn interpret_metadata(node: Node, meta_lineno: usize) -> Result<Metadata, FrontmatterError> {
    let entries = match node {
        Node::Map(e) => e,
        Node::Scalar(s) if s.is_empty() => Vec::new(),
        _ => {
            return Err(FrontmatterError::ExpectedMapping {
                key: "metadata".to_string(),
                line: meta_lineno,
            });
        }
    };
    let mut meta = Metadata::default();
    let mut have_tags = false;
    for MapEntry { key, lineno, value } in entries {
        match key.as_str() {
            "tags" => {
                have_tags = true;
                meta.tags = interpret_tags(value, lineno)?;
            }
            "triggers" => meta.triggers = Some(interpret_triggers(value, lineno)?),
            "lastReviewed" => {
                meta.last_reviewed = Some(expect_scalar(value, "lastReviewed", lineno)?)
            }
            "declineCount" => {
                let raw = expect_scalar(value, "declineCount", lineno)?;
                let n = raw.trim().parse::<i64>().map_err(|_| {
                    FrontmatterError::InvalidRankingField {
                        key: "declineCount".to_string(),
                        line: lineno,
                    }
                })?;
                meta.decline_count = Some(n);
            }
            other => {
                return Err(FrontmatterError::UnknownMetadataKey {
                    key: other.to_string(),
                    line: lineno,
                });
            }
        }
    }
    if !have_tags {
        return Err(FrontmatterError::MissingTags);
    }
    Ok(meta)
}

fn interpret_tags(node: Node, lineno: usize) -> Result<Vec<String>, FrontmatterError> {
    let items = match node {
        Node::Seq(v) => v,
        Node::Scalar(s) if s.is_empty() => return Err(FrontmatterError::EmptyTags),
        _ => {
            return Err(FrontmatterError::ExpectedSequence {
                key: "tags".to_string(),
                line: lineno,
            });
        }
    };
    if items.is_empty() {
        return Err(FrontmatterError::EmptyTags);
    }
    for t in &items {
        if !is_tag(t) {
            return Err(FrontmatterError::InvalidTag { tag: t.clone() });
        }
    }
    Ok(items)
}

fn interpret_triggers(node: Node, lineno: usize) -> Result<Triggers, FrontmatterError> {
    let entries = match node {
        Node::Map(e) => e,
        Node::Scalar(s) if s.is_empty() => Vec::new(),
        _ => {
            return Err(FrontmatterError::ExpectedMapping {
                key: "triggers".to_string(),
                line: lineno,
            });
        }
    };
    let mut tr = Triggers::default();
    for MapEntry { key, lineno, value } in entries {
        let slot = match key.as_str() {
            "commands" => &mut tr.commands,
            "paths" => &mut tr.paths,
            "args" => &mut tr.args,
            "synonyms" => &mut tr.synonyms,
            other => {
                return Err(FrontmatterError::UnknownTriggerKey {
                    key: other.to_string(),
                    line: lineno,
                });
            }
        };
        *slot = interpret_str_seq(value, &key, lineno)?;
    }
    Ok(tr)
}

fn interpret_str_seq(
    node: Node,
    key: &str,
    lineno: usize,
) -> Result<Vec<String>, FrontmatterError> {
    match node {
        Node::Seq(v) => Ok(v),
        Node::Scalar(s) if s.is_empty() => Ok(Vec::new()),
        _ => Err(FrontmatterError::ExpectedSequence {
            key: key.to_string(),
            line: lineno,
        }),
    }
}

fn expect_scalar(node: Node, key: &str, lineno: usize) -> Result<String, FrontmatterError> {
    match node {
        Node::Scalar(s) => Ok(s),
        _ => Err(FrontmatterError::ExpectedScalar {
            key: key.to_string(),
            line: lineno,
        }),
    }
}

// =============================================================================
// Differential-oracle rendering: tree -> stringified canonical JSON
// =============================================================================

fn node_to_json(node: &Node) -> serde_json::Value {
    match node {
        Node::Scalar(s) => serde_json::Value::String(s.clone()),
        Node::Seq(v) => serde_json::Value::Array(
            v.iter()
                .map(|s| serde_json::Value::String(s.clone()))
                .collect(),
        ),
        Node::Map(entries) => {
            // serde_json's default Map is key-sorted, matching PyYAML side's
            // json.dumps(sort_keys=True).
            let mut m = serde_json::Map::new();
            for e in entries {
                m.insert(e.key.clone(), node_to_json(&e.value));
            }
            serde_json::Value::Object(m)
        }
    }
}

// =============================================================================
// Generate: canonical serialization (round-trip oracle)
// =============================================================================

fn emit_flow_seq(items: &[String]) -> String {
    let mut out = String::from("[");
    for (idx, it) in items.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        out.push_str(&emit_scalar(it));
    }
    out.push(']');
    out
}

fn emit_scalar(s: &str) -> String {
    if is_safe_plain(s) {
        s.to_string()
    } else {
        emit_double_quoted(s)
    }
}

/// Conservative "can this be emitted as a bare plain scalar and re-parse to the
/// same string?" check. Over-quoting is safe (a double-quoted scalar always
/// re-parses); the goal is only to keep clean values (tags, plain paths)
/// unquoted for legibility.
fn is_safe_plain(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let first = s.chars().next().unwrap();
    if matches!(
        first,
        '*' | '&'
            | '!'
            | '|'
            | '>'
            | '['
            | ']'
            | '{'
            | '}'
            | '#'
            | ','
            | '%'
            | '@'
            | '\''
            | '"'
            | '?'
            | ':'
            | '-'
            | ' '
    ) {
        return false;
    }
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' | '/' | '~' | '+' | '=' | ':' => {}
            _ => return false,
        }
    }
    if s.contains(": ") {
        return false;
    }
    true
}

fn emit_double_quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_frontmatter_parses() {
        let fm = parse("---\nmetadata:\n  tags: [solo]\n---\nbody\n").unwrap();
        assert_eq!(fm.metadata.tags, vec!["solo".to_string()]);
        assert!(fm.name.is_none());
        assert!(fm.metadata.triggers.is_none());
    }

    #[test]
    fn round_trip_is_idempotent() {
        let src = "---\nname: gpu-notes\ndescription: GPU and VRAM diagnostics\nmetadata:\n  tags: [gpu, vram]\n  triggers:\n    commands: [nvidia-smi]\n    paths: [\"~/.config/gpu/**\"]\n    args: [\"--no-cache\"]\n    synonyms: [vram]\n---\n";
        let p1 = parse(src).unwrap();
        let g1 = generate(&p1);
        let p2 = parse(&g1).unwrap();
        assert_eq!(p1, p2);
        assert_eq!(g1, generate(&p2));
    }

    #[test]
    fn top_level_triggers_has_own_error() {
        let err =
            parse("---\ntriggers:\n  commands: [x]\nmetadata:\n  tags: [t]\n---\n").unwrap_err();
        assert!(matches!(err, FrontmatterError::TopLevelTriggers { .. }));
    }
}
