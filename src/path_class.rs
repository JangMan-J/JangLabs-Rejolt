//! The shared lexical path-specificity classifier (`is_broad_path`, §3.x).
//!
//! `is_broad_path` is the ONE place that decides whether a path trigger is *broad*
//! (a root/home/current-dir catchall with no domain signal) or *specific*. §3.x
//! makes it deliberately shared so a path cannot rescue the write-guard static gate
//! (§6) while being dead for collision projection (§7), or vice versa — the two
//! tiers read the SAME function, so they can never disagree about what "specific
//! path" means.
//!
//! It is **purely lexical** (§3.x): it does not stat the filesystem, resolve
//! symlinks, or consult the catalog. A path's classification depends only on its
//! text.
//!
//! ## The rule (§3.x, implemented EXACTLY)
//!
//! After normalization (trim; `~` / `$HOME` / `${HOME}` unified to one home
//! anchor; repeated `/` collapse and trailing `/` are handled by the segment scan;
//! `./foo` → `foo`), a path is **broad iff there is no concrete narrowing segment
//! before the first glob metacharacter** (`*`, `?`, `[`), ignoring empty, `.`, and
//! `..` segments. An anchor-only path (`/`, `~`, `.`) is broad. A **concrete
//! segment** is any non-empty segment that contains no glob metachar and is not `.`
//! or `..`.
//!
//! This diverges on purpose from a narrower classifier: `~/**/settings.json` and
//! `~/.*` are **broad** here (a glob metachar appears before any concrete segment),
//! matching how the flat-index walk ([`crate::index`]) already refuses to route a
//! leading/mid `**`. §3.x's example lists are the frozen conformance rows
//! (`tests/path_specificity.rs`).

/// Return `true` iff `raw` is a **broad** path under the §3.x lexical rule, `false`
/// iff it is **specific**. Pure: no filesystem access, no symlink resolution, no
/// catalog lookup.
///
/// Broad examples: `*`, `**`, `**/*.md`, `.`, `./**`, `../**`, `/`, `/*`, `/**`,
/// `~`, `~/`, `~/*`, `~/**`, `$HOME/**`, `${HOME}/*`, `~/.*`, `~/**/settings.json`.
///
/// Specific examples: `CORE-SPEC.md`, `src/**`, `./src/**`, `~/JangLabs/**`,
/// `~/.config/nvim/**`, `~/agent-projects/*/memory/*.md`, `/etc/modprobe.d/*.conf`,
/// `/var/log/pacman.log`.
pub fn is_broad_path(raw: &str) -> bool {
    let s = normalize_home_anchor(raw.trim());
    if s.is_empty() {
        return true; // nothing at all → broad
    }

    // Peel the leading anchor. Absolute root and the home anchor are NOT concrete
    // segments (an anchor-only path is broad); relative paths have no anchor and
    // the whole string is scanned as segments.
    let rest: &str = if s == "/" || s == "~" {
        return true; // anchor only
    } else if let Some(r) = s.strip_prefix('/') {
        r
    } else if let Some(r) = s.strip_prefix("~/") {
        r
    } else {
        &s
    };

    // Scan segments left-to-right. The empty / `.` / `..` segments are ignored
    // (this is what "collapse repeated `/`", "strip trailing `/`", and "drop
    // `./`" reduce to for classification). The first concrete segment makes the
    // path specific; the first glob metachar reached with no concrete segment
    // before it makes it broad.
    for seg in rest.split('/') {
        if seg.chars().any(is_glob_metachar) {
            return true; // glob before any concrete narrowing segment → broad
        }
        if seg.is_empty() || seg == "." || seg == ".." {
            continue; // not concrete, not a glob → keep scanning
        }
        return false; // a concrete narrowing segment before any glob → specific
    }
    // Only anchor / empty / `.` / `..` segments and no glob at all → broad.
    true
}

/// A glob metacharacter per §3.x: `*`, `?`, or `[`.
fn is_glob_metachar(c: char) -> bool {
    matches!(c, '*' | '?' | '[')
}

/// Unify the home anchor (§3.x rule 2): a **leading** `$HOME` or `${HOME}` (bare or
/// followed by `/`) becomes `~`, so the three spellings classify identically. A
/// non-anchor occurrence (`$HOMEDIR`, a mid-path `$HOME`) is left untouched.
fn normalize_home_anchor(s: &str) -> String {
    for anchor in ["${HOME}", "$HOME"] {
        if s == anchor {
            return "~".to_string();
        }
        if let Some(rest) = s.strip_prefix(anchor)
            && rest.starts_with('/')
        {
            return format!("~{rest}");
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every §3.x "must be broad" example, classified exactly.
    const BROAD: &[&str] = &[
        "*",
        "**",
        "**/*.md",
        ".",
        "./**",
        "../**",
        "/",
        "/*",
        "/**",
        "~",
        "~/",
        "~/*",
        "~/**",
        "$HOME/**",
        "${HOME}/*",
        "~/.*",
        "~/**/settings.json",
    ];

    /// Every §3.x "must be specific" example, classified exactly.
    const SPECIFIC: &[&str] = &[
        "CORE-SPEC.md",
        "src/**",
        "./src/**",
        "~/JangLabs/**",
        "~/.config/nvim/**",
        "~/agent-projects/*/memory/*.md",
        "/etc/modprobe.d/*.conf",
        "/var/log/pacman.log",
    ];

    #[test]
    fn every_section_3x_broad_example_is_broad() {
        for p in BROAD {
            assert!(is_broad_path(p), "§3.x says `{p}` MUST be broad");
        }
    }

    #[test]
    fn every_section_3x_specific_example_is_specific() {
        for p in SPECIFIC {
            assert!(!is_broad_path(p), "§3.x says `{p}` MUST be specific");
        }
    }

    #[test]
    fn recursive_catchall_after_concrete_prefix_is_specific() {
        // §3.x: "a recursive catchall after a concrete prefix is allowed
        // (`~/.config/**` is specific); a catchall before any concrete prefix is
        // broad (`~/**/settings.json` is broad)."
        assert!(!is_broad_path("~/.config/**"));
        assert!(is_broad_path("~/**/settings.json"));
    }

    #[test]
    fn normalization_collapses_slashes_and_trailing_and_dot() {
        // Repeated `/`, trailing `/`, and a leading `./` do not change the verdict.
        assert!(!is_broad_path("/a//b///c")); // concrete → specific
        assert!(!is_broad_path("src/**/")); // trailing slash irrelevant
        assert!(is_broad_path(" ~/ ")); // trimmed to `~/` → anchor only → broad
        assert!(!is_broad_path("./src/**")); // `./` dropped → specific
    }

    #[test]
    fn home_anchor_spellings_agree() {
        for (a, b) in [("~/**", "$HOME/**"), ("~/*", "${HOME}/*"), ("~", "$HOME")] {
            assert_eq!(
                is_broad_path(a),
                is_broad_path(b),
                "`{a}` and `{b}` must classify the same"
            );
        }
        // A non-anchor `$HOME` prefix is NOT unified.
        assert!(!is_broad_path("$HOMEDIR/config"));
    }
}
