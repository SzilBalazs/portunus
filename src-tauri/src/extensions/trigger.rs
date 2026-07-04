//! Trigger gating: decides whether an extension's `search` runs for a query,
//! and what query text it receives.
//!
//! Pure functions - the registry calls [`gate`] before spawning any extension
//! thread, so a gated-out extension costs nothing on the keystroke path.

use crate::extensions::manifest::TriggerConfig;

/// The query an extension receives after gating.
pub struct GatedQuery {
    /// Prefix-stripped query (equals the raw query in always-mode).
    pub query: String,
    /// The launcher query exactly as typed.
    pub raw_query: String,
    /// The prefix that matched, or None in always-mode.
    pub trigger: Option<String>,
}

/// Applies an extension's trigger config to the raw query. `None` = don't run.
///
/// Semantics:
/// - No `[trigger]` section: always-mode, raw query passed through (min length 1).
/// - Prefix match: first whitespace-separated token equals a prefix
///   (case-insensitive); the prefix and one following space are stripped.
///   `"emoji smi"` → `"smi"`; `"emoji"` / `"emoji "` → `""` (browse state).
/// - `min_query_len` applies to the *stripped* query, except that the bare
///   prefix (browse state) is always allowed through.
/// - `always = true` alongside prefixes: prefix match still strips; otherwise
///   the raw query is passed like a built-in provider.
pub fn gate(trigger: Option<&TriggerConfig>, raw_query: &str) -> Option<GatedQuery> {
    let Some(cfg) = trigger else {
        if raw_query.is_empty() {
            return None;
        }
        return Some(GatedQuery {
            query: raw_query.to_string(),
            raw_query: raw_query.to_string(),
            trigger: None,
        });
    };

    // split (not split_whitespace) so the token is anchored at byte 0 - the
    // slice below depends on that. A leading-space query yields "" = no match.
    let first_token = raw_query.split(char::is_whitespace).next().unwrap_or("");
    let matched = cfg
        .prefixes
        .iter()
        .find(|p| first_token.eq_ignore_ascii_case(p));

    if let Some(prefix) = matched {
        // Strip "<token>" plus at most one following space; anything beyond
        // that is the extension's query verbatim (leading spaces preserved
        // past the first - they typed it, the extension decides).
        let rest = &raw_query[first_token.len()..];
        let query = rest.strip_prefix(' ').unwrap_or(rest).to_string();
        // Bare prefix = browse state, always allowed; otherwise honor min len.
        if !query.is_empty() && query.len() < cfg.min_len() {
            return None;
        }
        return Some(GatedQuery {
            query,
            raw_query: raw_query.to_string(),
            trigger: Some(prefix.clone()),
        });
    }

    if cfg.always {
        if raw_query.is_empty() || raw_query.len() < cfg.min_len().max(1) {
            return None;
        }
        return Some(GatedQuery {
            query: raw_query.to_string(),
            raw_query: raw_query.to_string(),
            trigger: None,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(prefixes: &[&str], min: usize, always: bool) -> TriggerConfig {
        TriggerConfig {
            prefixes: prefixes.iter().map(|s| s.to_string()).collect(),
            min_query_len: min,
            always,
        }
    }

    #[test]
    fn no_trigger_is_always_mode() {
        let g = gate(None, "hello").unwrap();
        assert_eq!(g.query, "hello");
        assert!(g.trigger.is_none());
        assert!(gate(None, "").is_none());
    }

    #[test]
    fn prefix_strips_and_reports_trigger() {
        let c = cfg(&["emoji", "em"], 0, false);
        let g = gate(Some(&c), "emoji smi").unwrap();
        assert_eq!(g.query, "smi");
        assert_eq!(g.trigger.as_deref(), Some("emoji"));
    }

    #[test]
    fn bare_prefix_is_browse_state() {
        let c = cfg(&["emoji"], 3, false);
        assert_eq!(gate(Some(&c), "emoji").unwrap().query, "");
        assert_eq!(gate(Some(&c), "emoji ").unwrap().query, "");
    }

    #[test]
    fn min_len_applies_post_strip() {
        let c = cfg(&["emoji"], 3, false);
        assert!(gate(Some(&c), "emoji ab").is_none());
        assert!(gate(Some(&c), "emoji abc").is_some());
    }

    #[test]
    fn prefix_match_is_case_insensitive_and_token_exact() {
        let c = cfg(&["em"], 0, false);
        assert!(gate(Some(&c), "EM x").is_some());
        // "emoji" is not the token "em" - no match.
        assert!(gate(Some(&c), "emoji x").is_none());
    }

    #[test]
    fn non_matching_without_always_is_gated_out() {
        let c = cfg(&["emoji"], 0, false);
        assert!(gate(Some(&c), "firefox").is_none());
    }

    #[test]
    fn always_with_prefixes_passes_raw_query() {
        let c = cfg(&["emoji"], 0, true);
        let g = gate(Some(&c), "firefox").unwrap();
        assert_eq!(g.query, "firefox");
        assert!(g.trigger.is_none());
        // Prefix still strips when it matches.
        assert_eq!(gate(Some(&c), "emoji cat").unwrap().query, "cat");
    }

    #[test]
    fn extra_spaces_preserved_past_first() {
        let c = cfg(&["em"], 0, false);
        assert_eq!(gate(Some(&c), "em  spaced").unwrap().query, " spaced");
    }
}
