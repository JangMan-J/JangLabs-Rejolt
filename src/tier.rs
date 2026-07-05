//! The single type→tier map and the routing-axis / table / source vocabulary
//! (plan Appendix A; D5).
//!
//! Every flat-index column that is *derived* from a trigger's axis — the
//! `table`, `trigger_type`, and `tier` columns — is generated **here**, from one
//! [`Axis`] enum, so the three can never drift apart (Appendix A: "tier
//! precomputed at build from the hardcoded type→tier map — one module, column
//! generated from it"). Nothing else in the engine hardcodes command/path→strong
//! or arg→medium or synonym→weak; they read [`Axis::tier`].
//!
//! ## The D10 routing-vs-ranking partition tripwire
//!
//! The 13 columns split into three roles: **routing** (drives the matcher; a
//! change here must rebuild), **ranking** (scoring only; a change here must NOT
//! rebuild), and **display**. [`routing_ranking_partition_holds`] asserts the
//! routing and ranking column sets stay disjoint — the D10 invariant that a
//! field is never both. It is the guardrail hook (§11): if a future field
//! becomes both routing and ranking, it lands in both lists and the assertion
//! trips, surfacing the partition break as a drift advisory.

/// The flat-index schema version. Folded into the generation id (see
/// [`crate::catalog`]) so a schema bump invalidates every prior artifact pair —
/// an old index read against a new-schema report is a generation mismatch and is
/// caught as a stale pair (§4, A2d), never silently misread.
pub const SCHEMA_VERSION: u32 = 1;

/// The number of tab-separated columns in one flat-index record (plan
/// Appendix A). Load rejects any line without exactly this many columns.
pub const COLUMN_COUNT: usize = 13;

/// A routing axis — the four trigger kinds. It is 1:1 with an index table and a
/// [`Tier`]; the `table`, `trigger_type`, and `tier` columns are all rendered
/// from this single value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Axis {
    /// `byCommand` — command basenames. Strong tier.
    Command,
    /// `byPath` — path globs. Strong tier. The ONLY axis exempt from build-time
    /// key normalization (raw glob preserved; see [`crate::index`]).
    Path,
    /// `byArg` — argument tokens. Medium tier.
    Arg,
    /// `bySynonym` — synonym tokens. Weak tier.
    Synonym,
}

impl Axis {
    /// Every axis, in the canonical column/table order.
    pub const ALL: [Axis; 4] = [Axis::Command, Axis::Path, Axis::Arg, Axis::Synonym];

    /// The **hardcoded type→tier map** (Appendix A, D5): command/path = strong,
    /// arg = medium, synonym = weak. This is the one place the map lives.
    pub fn tier(self) -> Tier {
        match self {
            Axis::Command | Axis::Path => Tier::Strong,
            Axis::Arg => Tier::Medium,
            Axis::Synonym => Tier::Weak,
        }
    }

    /// The `table` column token (`byCommand` / `byPath` / `byArg` / `bySynonym`).
    pub fn table_str(self) -> &'static str {
        match self {
            Axis::Command => "byCommand",
            Axis::Path => "byPath",
            Axis::Arg => "byArg",
            Axis::Synonym => "bySynonym",
        }
    }

    /// The `trigger_type` column token (`command` / `path` / `arg` / `synonym`).
    /// This is the axis recall cites in `{route_tag} <- {trigger_type}:{value}`.
    pub fn trigger_type_str(self) -> &'static str {
        match self {
            Axis::Command => "command",
            Axis::Path => "path",
            Axis::Arg => "arg",
            Axis::Synonym => "synonym",
        }
    }

    /// Parse an [`Axis`] from a `table` column token. The load path keys off the
    /// `table` column and derives `trigger_type`/`tier` from it.
    pub fn from_table_str(s: &str) -> Option<Axis> {
        match s {
            "byCommand" => Some(Axis::Command),
            "byPath" => Some(Axis::Path),
            "byArg" => Some(Axis::Arg),
            "bySynonym" => Some(Axis::Synonym),
            _ => None,
        }
    }

    /// Whether this is the [`Axis::Path`] axis — the raw-glob, normalization-exempt
    /// table (Appendix A, A3).
    pub fn is_path(self) -> bool {
        matches!(self, Axis::Path)
    }
}

/// A routing tier. Precomputed into the `tier` column from [`Axis::tier`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// command / path.
    Strong,
    /// arg.
    Medium,
    /// synonym.
    Weak,
}

impl Tier {
    /// The `tier` column token.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Strong => "strong",
            Tier::Medium => "medium",
            Tier::Weak => "weak",
        }
    }
}

/// The provenance of a routing row (`source` column). Grammar-tag routes and
/// per-memory triggers fold into the SAME four tables; `source` distinguishes
/// them and fixes what `route_tag` means (Appendix A, A2c).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// `t` — a grammar-tag route. `route_tag` is the grammar tag name (what
    /// recall citations, tag filtering, and telemetry attribution consume).
    Tag,
    /// `m` — a per-memory trigger. `route_tag` is the memory id.
    Memory,
}

impl Source {
    /// The `source` column token (`t` / `m`).
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Tag => "t",
            Source::Memory => "m",
        }
    }

    /// Parse a [`Source`] from its column token (`t` / `m`).
    pub fn from_token(s: &str) -> Option<Source> {
        match s {
            "t" => Some(Source::Tag),
            "m" => Some(Source::Memory),
            _ => None,
        }
    }
}

// =============================================================================
// D10 routing-vs-ranking partition (§4, §11 tripwire)
// =============================================================================

/// The columns whose value the matcher routes on. A change to any of these is a
/// **routing** change and must trigger a rebuild (D10, §4).
pub const ROUTING_COLUMNS: &[&str] = &[
    "table",
    "pattern",
    "route_tag",
    "source",
    "trigger_type",
    "tier",
];

/// The columns that affect scoring only. A change to one of these is a
/// **ranking-only** write and must NOT trigger a rebuild (D10, §4) — recall
/// re-reads them from the existing index without a rebuild.
pub const RANKING_COLUMNS: &[&str] = &["lastReviewed", "declineCount"];

/// The D10 partition invariant: routing and ranking column sets are disjoint.
///
/// Trivially true today. It is the §11 tripwire: if a future field becomes both
/// routing- and ranking-affecting, it is added to both lists and this returns
/// `false`, so [`crate::rebuild::drift_guardrail`] surfaces the partition break
/// as an advisory rather than letting a ranking write silently skip a rebuild it
/// now needs.
pub fn routing_ranking_partition_holds() -> bool {
    ROUTING_COLUMNS.iter().all(|r| !RANKING_COLUMNS.contains(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_to_tier_map_is_frozen() {
        assert_eq!(Axis::Command.tier(), Tier::Strong);
        assert_eq!(Axis::Path.tier(), Tier::Strong);
        assert_eq!(Axis::Arg.tier(), Tier::Medium);
        assert_eq!(Axis::Synonym.tier(), Tier::Weak);
    }

    #[test]
    fn table_and_trigger_type_tokens_round_trip() {
        for axis in Axis::ALL {
            assert_eq!(Axis::from_table_str(axis.table_str()), Some(axis));
        }
        assert_eq!(Axis::Command.trigger_type_str(), "command");
        assert_eq!(Axis::Path.table_str(), "byPath");
        assert!(Axis::Path.is_path());
        assert!(!Axis::Command.is_path());
    }

    #[test]
    fn source_tokens_round_trip() {
        assert_eq!(Source::from_token(Source::Tag.as_str()), Some(Source::Tag));
        assert_eq!(
            Source::from_token(Source::Memory.as_str()),
            Some(Source::Memory)
        );
        assert_eq!(Source::from_token("x"), None);
    }

    #[test]
    fn d10_partition_holds_today() {
        assert!(routing_ranking_partition_holds());
    }
}
