//! The shared tag-shape predicate (`TAG_RE`).
//!
//! A routing tag is written in **kebab-case**. This is the single definition of
//! that shape, kept in one module so every later packet that needs it —
//! frontmatter schema validation (WP-1), build-time index key normalization and
//! `TAG_RE` conformance (WP-2, plan Appendix A), and byArg/bySynonym liveness
//! (WP-4) — reuses exactly this predicate instead of re-deriving it. Keeping the
//! rule here (not inlined in the frontmatter parser) is what makes that reuse
//! honest.
//!
//! The shape forbids commas by construction, which is load-bearing: the flat
//! recall index joins a memory's tags with `,` on one physical line (plan
//! Appendix A), so a comma in a tag would corrupt the column.

/// Regex-equivalent description of the kebab-case tag shape enforced by
/// [`is_tag`]. Kept as a `pub const` so callers and diagnostics can cite the
/// exact pattern rather than paraphrasing it.
///
/// Equivalent to the anchored regex `^[a-z0-9]+(-[a-z0-9]+)*$`: one or more
/// lowercase-alphanumeric segments joined by single hyphens, with no leading,
/// trailing, or doubled hyphen and no other characters.
pub const TAG_PATTERN: &str = "^[a-z0-9]+(-[a-z0-9]+)*$";

/// Returns `true` iff `s` is a valid kebab-case tag under [`TAG_PATTERN`].
///
/// Accepts: `gpu`, `gpu-tools`, `nvidia-smi`, `a1`, `x-y-z`.
/// Rejects: empty, `GPU` (uppercase), `-lead`, `trail-`, `double--dash`,
/// `has_underscore`, `has space`, `has,comma`.
pub fn is_tag(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    // Leading/trailing hyphen is invalid; caught here so the loop can focus on
    // the "no doubled hyphen, only allowed chars" invariant.
    if s.starts_with('-') || s.ends_with('-') {
        return false;
    }
    let mut prev_dash = false;
    for c in s.chars() {
        match c {
            'a'..='z' | '0'..='9' => prev_dash = false,
            '-' => {
                if prev_dash {
                    return false; // no doubled hyphen
                }
                prev_dash = true;
            }
            _ => return false, // any other char (uppercase, '_', ',', space, …)
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_kebab() {
        for s in [
            "gpu",
            "gpu-tools",
            "nvidia-smi",
            "a1",
            "x-y-z",
            "0",
            "a-1-b",
        ] {
            assert!(is_tag(s), "should accept `{s}`");
        }
    }

    #[test]
    fn rejects_non_kebab() {
        for s in [
            "",
            "GPU",
            "-lead",
            "trail-",
            "double--dash",
            "has_underscore",
            "has space",
            "has,comma",
            "UPPER-case",
            "dot.ted",
        ] {
            assert!(!is_tag(s), "should reject `{s}`");
        }
    }
}
