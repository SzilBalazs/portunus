//! Command catalog: the descriptors behind the launcher's searchable command
//! entries ("Define Word", "Clipboard History", extension commands). A
//! descriptor is pure data - execution always flows through the owning
//! provider/extension; the command layer only catalogs and routes.

use serde::Serialize;

use super::{fuzzy_best, fuzzy_bonus, fuzzy_setup, quality_threshold, SearchResult, SCORE_COMMAND};

/// How a command behaves when invoked.
// Action is constructed once extensions declare action commands.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeKind {
    /// Enter-able scope: a chip appears and all typing routes to the command.
    Scope,
    /// One-shot: Enter runs the command's activation and closes.
    Action,
}

// Extension is constructed once extensions declare commands (api 4).
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandSource {
    Builtin,
    Extension { name: String },
}

/// Where a scoped search for this command is executed.
// Extension is constructed once extensions declare commands (api 4).
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandRoute {
    /// A built-in provider's `search_scoped` handles the query.
    Builtin { provider_id: String },
    /// An extension command handles the query (`command` on the wire).
    Extension { name: String, command: String },
    /// The frontend swaps in a dedicated component; the backend is not called.
    UiTakeover,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandDescriptor {
    /// Stable identity. Built-ins: `cmd:<name>`. Extension commands:
    /// `ext:<name>:cmd:<command>` - the `ext:<name>:` prefix keeps them inside
    /// the frecency uninstall sweep.
    pub id: String,
    /// Root-search entry title ("Search Issues").
    pub title: String,
    /// Short label for the active-mode chip ("Contents", "Issues").
    pub chip: String,
    /// Right-side context shown on the entry row ("GitHub", "Dictionary").
    pub subtitle: Option<String>,
    pub source: CommandSource,
    pub mode_kind: ModeKind,
    /// Search synonyms folded into the fuzzy match alongside the title
    /// ("define", "dictionary"). Pure search terms - they never gate or trigger.
    pub keywords: Vec<String>,
    /// Input placeholder while the scope is active.
    pub placeholder: Option<String>,
    /// Minimum chars before the scoped search runs.
    pub min_query_len: usize,
    /// `SearchResult.kind` the command's own results carry.
    pub result_kind: String,
    /// Built-in command icon: a named glyph the frontend renders inline (so it
    /// follows the theme's `currentColor`). Extension commands leave this None
    /// and ship a raster icon via `icon_data_uri` instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_data_uri: Option<String>,
    /// Action commands only: the command opens a form on activation, so the
    /// frontend must not hide the launcher optimistically while it runs.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub opens_form: bool,
    pub route: CommandRoute,
}

/// Minimum fuzzy quality a command entry needs before it surfaces, fed to the
/// shared `quality_threshold` ramp (same scale as the apps provider's
/// `min_quality`, default 0.06). Held slightly stricter than apps because a
/// matched command entry outranks every app (base 4.5M vs 2M): calibrated so a
/// word-boundary prefix ("sea"→"Search Emoji", "def"→"define") passes while a
/// scattered subsequence ("set"→"Search File conTents") is rejected and cannot
/// bury the app the user is actually typing.
const COMMAND_MIN_QUALITY: f32 = 0.07;

/// Projects the command entries matching `query` into result rows.
///
/// A command is a first-class fuzzy target, ranked exactly like an app: the
/// query is matched against `[title, ...keywords, subtitle]` (best field wins,
/// subtitle down-weighted) via the shared nucleo machinery. No prefix, alias,
/// or exact-token special-casing - `keywords` are just extra haystacks.
pub fn match_entries(commands: &[CommandDescriptor], query: &str) -> Vec<SearchResult> {
    let q = query.trim();
    // Sub-2-char queries match almost everything; hold entries back until there
    // is a real term (apps/files tolerate this via their own length ramp).
    if q.chars().count() < 2 {
        return Vec::new();
    }

    let (pattern, mut matcher, mut buf) = fuzzy_setup(q);
    let threshold = quality_threshold(COMMAND_MIN_QUALITY, q.chars().count());

    commands
        .iter()
        .filter_map(|cmd| {
            let mut fields: Vec<(&str, f32)> = Vec::with_capacity(2 + cmd.keywords.len());
            fields.push((cmd.title.as_str(), 1.0));
            fields.extend(cmd.keywords.iter().map(|k| (k.as_str(), 1.0)));
            if let Some(sub) = &cmd.subtitle {
                fields.push((sub.as_str(), 0.8));
            }
            let (_, score) = fuzzy_best(&pattern, &mut matcher, &mut buf, &fields)?;
            if (score as f32) < threshold {
                return None;
            }
            Some(SearchResult {
                id: cmd.id.clone(),
                title: cmd.title.clone(),
                subtitle: cmd.subtitle.clone(),
                kind: "command".to_string(),
                score: SCORE_COMMAND + fuzzy_bonus(score),
                icon_data_uri: cmd.icon_data_uri.clone(),
                command: Some(cmd.clone()),
                ..Default::default()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(id: &str, title: &str, keywords: &[&str]) -> CommandDescriptor {
        CommandDescriptor {
            id: id.to_string(),
            title: title.to_string(),
            chip: title.to_string(),
            subtitle: None,
            source: CommandSource::Builtin,
            mode_kind: ModeKind::Scope,
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            placeholder: None,
            min_query_len: 1,
            result_kind: "dict".to_string(),
            glyph: None,
            icon_data_uri: None,
            opens_form: false,
            route: CommandRoute::Builtin { provider_id: "dict".to_string() },
        }
    }

    #[test]
    fn keyword_match_surfaces_entry() {
        let cmds = vec![cmd("cmd:dict", "Define Word", &["define", "dict"])];
        let hits = match_entries(&cmds, "dict");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "cmd:dict");
        assert!(hits[0].score >= SCORE_COMMAND);
    }

    #[test]
    fn keyword_prefix_matches() {
        let cmds = vec![cmd("cmd:dict", "Define Word", &["define", "dict"])];
        assert!(!match_entries(&cmds, "def").is_empty());
    }

    #[test]
    fn title_fuzzy_matches() {
        let cmds = vec![cmd("cmd:dict", "Define Word", &["define"])];
        assert!(!match_entries(&cmds, "word").is_empty());
    }

    #[test]
    fn keyword_with_args_still_matches() {
        // No alias suppression anymore: the entry is a plain fuzzy target.
        let cmds = vec![cmd("cmd:dict", "Define Word", &["define", "dict"])];
        // "define" (a keyword) fuzzy-matches even with trailing text.
        assert!(!match_entries(&cmds, "define").is_empty());
    }

    #[test]
    fn weak_subsequence_does_not_match() {
        let cmds = vec![cmd("cmd:contents", "Search File Contents", &["contents"])];
        assert!(match_entries(&cmds, "fir").is_empty());
    }

    #[test]
    fn scattered_subsequence_rejected() {
        // "set" is a scattered subsequence of "Search File conTents"; it must
        // not surface the command (and outrank a real "Settings" app).
        let cmds = vec![cmd("cmd:contents", "Search File Contents", &["contents"])];
        assert!(match_entries(&cmds, "set").is_empty());
    }

    #[test]
    fn word_boundary_prefix_matches() {
        // A real prefix of a title word surfaces the entry.
        let cmds = vec![cmd("ext:emoji:cmd:search", "Search Emoji", &["emoji"])];
        assert!(!match_entries(&cmds, "sea").is_empty());
    }

    #[test]
    fn short_query_never_matches() {
        let cmds = vec![cmd("cmd:dict", "Define Word", &["define"])];
        assert!(match_entries(&cmds, "d").is_empty());
    }
}
