//! The `grammar.toml` loader (D22, D23, D3, A6) — the routing-vocabulary
//! scoping gate, carrying **zero bespoke parsing**.
//!
//! Where the frontmatter dialect ([`crate::frontmatter`]) is hand-rolled by
//! necessity, the grammar file is the opposite decision (D23): it is the `toml`
//! crate + serde into typed structs. Keeping the grammar *out* of the bespoke
//! parser is what shrinks the one hand-rolled surface to exactly one.
//!
//! ## What the type system enforces (D22/D23/A6a)
//!
//! - `#[serde(deny_unknown_fields)]` on the root makes the facet set **closed**:
//!   the only top-level tables are `domain` / `tool` / `pattern`, so a fourth
//!   table name is a *deserialization error* (exit-2 class) — D22's closed set,
//!   structurally enforced (A6a).
//! - `#[serde(deny_unknown_fields)]` on every [`Entry`] rejects stray fields.
//! - `placement` is an enum, so a bad placement string is a deserialization
//!   error rather than a silently-accepted value.
//!
//! ## What the engine must check on top (serde cannot)
//!
//! - **One facet per tag (A6a):** a tag name under more than one facet table is
//!   legal TOML, so [`validate_grammar`] cross-checks it.
//! - **Evidence non-empty (D3/D23):** a present tag whose command+path+arg
//!   evidence is all empty fails (synonyms alone are insufficient). The **empty
//!   seed** (version line alone, zero entries) is the only zero-evidence file
//!   that passes — it has no present tag to fail.
//! - **`grammar-version` validated (D23):** required by the type, and its value
//!   must be `1`.
//!
//! Every [`GrammarError`] is classifiable as config/taxonomy → the later
//! `validate` CLI (WP-7) maps them to exit 2 via [`GrammarError::exit_code`].
//! This module wires no CLI; it exposes clean library functions.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Deserialize;

/// The only accepted `grammar-version`.
pub const GRAMMAR_VERSION: i64 = 1;

/// A parsed, not-yet-validated grammar. Deserializes from `grammar.toml`.
///
/// `deny_unknown_fields` here is load-bearing: it is what makes a fourth facet
/// table (A6a / D22) a hard deserialization error.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grammar {
    /// Required, must equal [`GRAMMAR_VERSION`] (validated in
    /// [`validate_grammar`]).
    #[serde(rename = "grammar-version")]
    pub grammar_version: i64,
    /// `[domain.<tag>]` entries.
    #[serde(default)]
    pub domain: BTreeMap<String, Entry>,
    /// `[tool.<tag>]` entries.
    #[serde(default)]
    pub tool: BTreeMap<String, Entry>,
    /// `[pattern.<tag>]` entries.
    #[serde(default)]
    pub pattern: BTreeMap<String, Entry>,
}

/// A single grammar tag entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// One-line human gloss (validated non-empty).
    pub gloss: String,
    /// Placement hint. Modeled as an enum so a bad value is a deser error.
    pub placement: Placement,
    /// Command evidence (strong tier).
    #[serde(default)]
    pub commands: Vec<String>,
    /// Path evidence (strong tier).
    #[serde(default)]
    pub paths: Vec<String>,
    /// Arg-token evidence (medium tier).
    #[serde(default)]
    pub args: Vec<String>,
    /// Synonym evidence (weak tier) — insufficient alone (D3).
    #[serde(default)]
    pub synonyms: Vec<String>,
    /// Related tag names (advisory; no routing role).
    #[serde(default)]
    pub related: Vec<String>,
}

impl Entry {
    /// Whether this entry carries any behavioral (command/path/arg) evidence.
    /// Synonyms and `related` do not count (D3). Only NON-BLANK entries count: a
    /// blank/whitespace-only token is not a usable route key, so `commands=[""]`
    /// does not satisfy the D3 "a tag is its evidence" guard.
    fn has_behavioral_evidence(&self) -> bool {
        let nonblank = |v: &[String]| v.iter().any(|s| !s.trim().is_empty());
        nonblank(&self.commands) || nonblank(&self.paths) || nonblank(&self.args)
    }
}

/// A tag's store-placement hint (D23). `rename_all = "lowercase"` maps the
/// variants to the TOML tokens `box` / `project` / `either`; any other string is
/// a deserialization error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Placement {
    /// Box-brain store.
    Box,
    /// A project store.
    Project,
    /// Either store.
    Either,
}

impl Placement {
    /// The lowercase token form, for rendering.
    pub fn as_str(self) -> &'static str {
        match self {
            Placement::Box => "box",
            Placement::Project => "project",
            Placement::Either => "either",
        }
    }
}

impl fmt::Display for Placement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A grammar load/validation failure. Every variant is a config/taxonomy error
/// (exit-2 class); see [`GrammarError::exit_code`].
#[derive(Debug, Clone)]
pub enum GrammarError {
    /// A TOML deserialization failure. This is the exit-2 surface for the
    /// structurally-enforced rules: a fourth facet table, an unknown entry
    /// field, a bad `placement` value, and a missing `grammar-version` all land
    /// here. The string is the underlying serde/toml message.
    Parse(String),
    /// `grammar-version` deserialized but is not [`GRAMMAR_VERSION`].
    UnsupportedVersion(i64),
    /// A tag appears under more than one facet table (A6a).
    DuplicateFacet {
        /// The offending tag name.
        tag: String,
        /// The facets it appears under, sorted.
        facets: Vec<&'static str>,
    },
    /// A present tag has empty command+path+arg evidence (D3/D23).
    NoEvidence {
        /// Facet the tag lives under.
        facet: &'static str,
        /// The offending tag name.
        tag: String,
    },
    /// A present tag has an empty `gloss` (D23).
    EmptyGloss {
        /// Facet the tag lives under.
        facet: &'static str,
        /// The offending tag name.
        tag: String,
    },
    /// An evidence entry (in `commands`/`paths`/`args`/`synonyms`) is blank
    /// after trimming or contains a control char (`\t`/`\n`/`\r`) — it is not a
    /// usable route key and would corrupt the one-line flat index.
    InvalidEvidence {
        /// Facet the tag lives under.
        facet: &'static str,
        /// The offending tag name.
        tag: String,
        /// The evidence field the bad entry is in.
        field: &'static str,
    },
    /// A `gloss` contains a newline/CR. Gloss is one line (CORE-SPEC §3); a
    /// multiline gloss would inject fake lines into the rendered digest.
    MultilineGloss {
        /// Facet the tag lives under.
        facet: &'static str,
        /// The offending tag name.
        tag: String,
    },
    /// A facet tag NAME is not `TAG_RE`-shaped (kebab-case; D21/D22, plan
    /// Appendix A "TAG_RE conformance at validate"). A non-kebab grammar tag is
    /// dead vocabulary — memory `metadata.tags` are kebab-enforced, so nothing
    /// can ever be a member of it (walk-back fix F16, 2026-07-04).
    InvalidTagName {
        /// Facet the tag lives under.
        facet: &'static str,
        /// The offending tag name.
        tag: String,
    },
}

impl GrammarError {
    /// The process exit code the `validate` CLI (WP-7) maps this to. Every
    /// grammar error is a config/taxonomy error → exit 2 (D20/A5 taxonomy).
    pub fn exit_code(&self) -> i32 {
        2
    }
}

impl fmt::Display for GrammarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrammarError::Parse(msg) => write!(f, "grammar parse error: {msg}"),
            GrammarError::UnsupportedVersion(v) => write!(
                f,
                "unsupported grammar-version {v} (expected {GRAMMAR_VERSION})"
            ),
            GrammarError::DuplicateFacet { tag, facets } => write!(
                f,
                "tag `{tag}` appears under more than one facet: {}",
                facets.join(", ")
            ),
            GrammarError::NoEvidence { facet, tag } => write!(
                f,
                "tag `{facet}.{tag}` has no behavioral evidence (commands/paths/args); synonyms alone are insufficient"
            ),
            GrammarError::EmptyGloss { facet, tag } => {
                write!(f, "tag `{facet}.{tag}` has an empty gloss")
            }
            GrammarError::InvalidEvidence { facet, tag, field } => write!(
                f,
                "tag `{facet}.{tag}` has a blank or control-char `{field}` evidence entry"
            ),
            GrammarError::MultilineGloss { facet, tag } => {
                write!(
                    f,
                    "tag `{facet}.{tag}` has a multiline gloss (must be one line)"
                )
            }
            GrammarError::InvalidTagName { facet, tag } => write!(
                f,
                "tag name `{facet}.{tag}` is not kebab-case (TAG_RE) — memory tags are \
                 kebab-enforced, so this tag can never have a member"
            ),
        }
    }
}

impl std::error::Error for GrammarError {}

/// Deserialize `grammar.toml` text into a [`Grammar`] without validation. The
/// structural rules (closed facet set, unknown fields, `placement` domain,
/// required `grammar-version`) are enforced here by serde.
pub fn parse_grammar(text: &str) -> Result<Grammar, GrammarError> {
    toml::from_str(text).map_err(|e| GrammarError::Parse(e.to_string()))
}

/// Run the engine-side validation serde cannot: version value, one-facet-per-tag
/// (A6a), non-empty gloss, and non-empty behavioral evidence (D3/D23). The empty
/// seed (zero entries) passes.
pub fn validate_grammar(g: &Grammar) -> Result<(), GrammarError> {
    if g.grammar_version != GRAMMAR_VERSION {
        return Err(GrammarError::UnsupportedVersion(g.grammar_version));
    }

    // One facet per tag (A6a): serde permits the same name under two tables.
    let mut facets_by_tag: BTreeMap<&str, Vec<&'static str>> = BTreeMap::new();
    for name in g.domain.keys() {
        facets_by_tag.entry(name).or_default().push("domain");
    }
    for name in g.tool.keys() {
        facets_by_tag.entry(name).or_default().push("tool");
    }
    for name in g.pattern.keys() {
        facets_by_tag.entry(name).or_default().push("pattern");
    }
    for (tag, facets) in &facets_by_tag {
        if facets.len() > 1 {
            let mut facets = facets.clone();
            facets.sort();
            return Err(GrammarError::DuplicateFacet {
                tag: (*tag).to_string(),
                facets,
            });
        }
    }

    // Per-entry gloss + evidence (D3/D23). Deterministic order.
    for (facet, map) in [
        ("domain", &g.domain),
        ("tool", &g.tool),
        ("pattern", &g.pattern),
    ] {
        for (tag, entry) in map {
            // Tag NAME shape (F16; Appendix A "TAG_RE conformance at validate"):
            // a non-kebab tag name is dead vocabulary — kebab-enforced memory
            // tags can never be a member of it.
            if !crate::tag::is_tag(tag) {
                return Err(GrammarError::InvalidTagName {
                    facet,
                    tag: tag.clone(),
                });
            }
            if entry.gloss.trim().is_empty() {
                return Err(GrammarError::EmptyGloss {
                    facet,
                    tag: tag.clone(),
                });
            }
            if entry.gloss.contains(['\n', '\r']) {
                return Err(GrammarError::MultilineGloss {
                    facet,
                    tag: tag.clone(),
                });
            }
            // No blank / control-char evidence entry, in any array. A blank key
            // never routes; a `\t`/`\n`/`\r` would break the flat index's
            // one-record-per-line invariant.
            for (field, arr) in [
                ("commands", &entry.commands),
                ("paths", &entry.paths),
                ("args", &entry.args),
                ("synonyms", &entry.synonyms),
            ] {
                for v in arr {
                    if v.trim().is_empty() || v.contains(['\t', '\n', '\r']) {
                        return Err(GrammarError::InvalidEvidence {
                            facet,
                            tag: tag.clone(),
                            field,
                        });
                    }
                }
            }
            if !entry.has_behavioral_evidence() {
                return Err(GrammarError::NoEvidence {
                    facet,
                    tag: tag.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Parse and validate in one step — the primary loader entry point.
pub fn parse_and_validate(text: &str) -> Result<Grammar, GrammarError> {
    let g = parse_grammar(text)?;
    validate_grammar(&g)?;
    Ok(g)
}

/// The FULL set of validation-error signatures for a grammar text — every error,
/// not just the first (unlike [`validate_grammar`], which fails fast). Each
/// signature is a stable, content-derived string (e.g. `dup-facet:gpu`,
/// `no-evidence:domain.gpu`); a text that fails to even parse yields the single
/// coarse signature `parse` (a parse failure is one blocking error and cannot be
/// enumerated further).
///
/// This is the A6 diff-aware grammar-write surface's building block
/// ([`crate::guard`]): a full-file grammar write is denied only when it introduces
/// a signature the CURRENT file does not already have. Because it enumerates ALL
/// errors, the set-difference is sound — an edit that FIXES one pre-existing error
/// while leaving another can never be mistaken for introducing a new one (which a
/// first-error-only comparison would false-deny — the #1-rule violation).
pub fn error_signatures(text: &str) -> BTreeSet<String> {
    let g = match parse_grammar(text) {
        Ok(g) => g,
        // A deserialization failure (fourth facet table, unknown field, bad
        // placement, missing version) is a single blocking error.
        Err(_) => return BTreeSet::from(["parse".to_string()]),
    };
    let mut sigs = BTreeSet::new();
    if g.grammar_version != GRAMMAR_VERSION {
        sigs.insert(format!("version:{}", g.grammar_version));
    }
    // One facet per tag (A6a) — collect EVERY duplicate, not just the first.
    let mut facets_by_tag: BTreeMap<&str, usize> = BTreeMap::new();
    for map in [&g.domain, &g.tool, &g.pattern] {
        for name in map.keys() {
            *facets_by_tag.entry(name.as_str()).or_default() += 1;
        }
    }
    for (tag, count) in &facets_by_tag {
        if *count > 1 {
            sigs.insert(format!("dup-facet:{tag}"));
        }
    }
    // Per-entry gloss + evidence, mirroring [`validate_grammar`]'s checks exactly.
    for (facet, map) in [
        ("domain", &g.domain),
        ("tool", &g.tool),
        ("pattern", &g.pattern),
    ] {
        for (tag, entry) in map {
            if !crate::tag::is_tag(tag) {
                sigs.insert(format!("bad-tag-name:{facet}.{tag}"));
            }
            if entry.gloss.trim().is_empty() {
                sigs.insert(format!("empty-gloss:{facet}.{tag}"));
            }
            if entry.gloss.contains(['\n', '\r']) {
                sigs.insert(format!("multiline-gloss:{facet}.{tag}"));
            }
            for (field, arr) in [
                ("commands", &entry.commands),
                ("paths", &entry.paths),
                ("args", &entry.args),
                ("synonyms", &entry.synonyms),
            ] {
                for v in arr {
                    if v.trim().is_empty() || v.contains(['\t', '\n', '\r']) {
                        sigs.insert(format!("invalid-evidence:{facet}.{tag}.{field}"));
                    }
                }
            }
            if !entry.has_behavioral_evidence() {
                sigs.insert(format!("no-evidence:{facet}.{tag}"));
            }
        }
    }
    sigs
}

/// Render the write-context vocabulary digest **from the parsed data** (D23):
/// doc-quality is the renderer's property, decoupling storage format from prompt
/// text. Deterministic (facets in a fixed order, tags key-sorted). Later packets
/// (WP-4 write-context) inject this. The empty seed renders as the header alone.
pub fn render_digest(g: &Grammar) -> String {
    let mut out = format!(
        "# Trigger vocabulary — grammar-version {}\n",
        g.grammar_version
    );
    for (facet, map) in [
        ("domain", &g.domain),
        ("tool", &g.tool),
        ("pattern", &g.pattern),
    ] {
        for (tag, entry) in map {
            out.push_str(&format!(
                "{facet}/{tag} [{}]: {}\n",
                entry.placement,
                entry.gloss.trim()
            ));
            push_evidence_line(&mut out, "commands", &entry.commands);
            push_evidence_line(&mut out, "paths", &entry.paths);
            push_evidence_line(&mut out, "args", &entry.args);
            push_evidence_line(&mut out, "synonyms", &entry.synonyms);
        }
    }
    out
}

fn push_evidence_line(out: &mut String, label: &str, values: &[String]) {
    if !values.is_empty() {
        out.push_str(&format!("  {label}: {}\n", values.join(", ")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_seed_is_valid() {
        let g = parse_and_validate("grammar-version = 1\n").unwrap();
        assert!(g.domain.is_empty() && g.tool.is_empty() && g.pattern.is_empty());
    }

    #[test]
    fn empty_seed_digest_is_header_only() {
        let g = parse_and_validate("grammar-version = 1\n").unwrap();
        assert_eq!(render_digest(&g).lines().count(), 1);
    }

    #[test]
    fn synonyms_only_fails() {
        let toml = "grammar-version = 1\n\n[domain.weak]\ngloss = \"w\"\nplacement = \"either\"\nsynonyms = [\"foo\"]\n";
        assert!(matches!(
            parse_and_validate(toml),
            Err(GrammarError::NoEvidence { .. })
        ));
    }
}
