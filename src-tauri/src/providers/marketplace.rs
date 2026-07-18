//! Marketplace scope provider: browses/searches the cached extension index.
//!
//! Root search only surfaces the "Browse Extension Marketplace" command entry;
//! entering the scope lists the catalog (empty query = browse everything).
//! Search is synchronous over the in-memory index cache - the network fetch
//! happens off-path (startup thread / scope-entry refresh command).

use std::sync::Arc;

use serde::Serialize;

use super::{fuzzy_best, fuzzy_setup, CommandDescriptor, Provider, SearchResult, SCORE_MARKETPLACE};
use crate::extensions::marketplace::{self as market, IndexEntry, Store};
use crate::extensions::{extensions_dir, install, manifest};

/// Update rows sort above the rest of the catalog.
const UPDATE_BONUS: f32 = 200_000.0;

/// Frontend payload for a `kind: "marketplace"` row - everything the preview
/// panel needs to act as the consent surface, plus install-state.
#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceResult {
    pub name: String,
    pub version: String,
    pub api: u32,
    pub description: String,
    pub author: String,
    pub homepage: String,
    pub keywords: Vec<String>,
    pub permissions: install::ConsentPermissions,
    pub size_bytes: u64,
    /// "not_installed" | "installed" | "update" | "incompatible"
    pub state: String,
    pub installed_version: Option<String>,
    /// Update rows: the new version asks for more than the consented snapshot.
    pub permissions_grew: bool,
    /// A same-name dev symlink exists; marketplace install is blocked.
    pub dev_conflict: bool,
}

pub struct MarketplaceProvider {
    store: Arc<Store>,
}

impl MarketplaceProvider {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    fn row(&self, entry: &IndexEntry, consents: &std::collections::HashMap<String, install::ConsentRecord>) -> SearchResult {
        let dir = extensions_dir().join(&entry.name);
        let dev_conflict =
            std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_symlink());
        let installed_version = if dir.is_dir() {
            manifest::load(&dir).map(|(m, _)| m.version).ok().or_else(|| {
                // Broken manifest on disk still counts as installed.
                consents.get(&entry.name).map(|r| r.version.clone())
            })
        } else {
            None
        };
        let state = match &installed_version {
            Some(v) if market::version_newer(&entry.version, v) && !dev_conflict => "update",
            Some(_) => "installed",
            None if entry.api != manifest::SUPPORTED_API => "incompatible",
            None => "not_installed",
        };
        let permissions_grew = state == "update"
            && consents
                .get(&entry.name)
                .map(|r| r.permissions.grew_to(&entry.permissions))
                .unwrap_or(false);

        SearchResult {
            id: format!("market:{}", entry.name),
            title: entry.name.clone(),
            subtitle: if entry.description.is_empty() {
                None
            } else {
                Some(entry.description.clone())
            },
            kind: "marketplace".to_string(),
            // Base score; the caller adds fuzzy/browse components.
            score: SCORE_MARKETPLACE + if state == "update" { UPDATE_BONUS } else { 0.0 },
            icon_data_uri: entry.icon_data_uri.clone(),
            market: Some(MarketplaceResult {
                name: entry.name.clone(),
                version: entry.version.clone(),
                api: entry.api,
                description: entry.description.clone(),
                author: entry.author.clone(),
                homepage: entry.homepage.clone(),
                keywords: entry.keywords.clone(),
                permissions: entry.permissions.clone(),
                size_bytes: entry.size_bytes,
                state: state.to_string(),
                installed_version,
                permissions_grew,
                dev_conflict,
            }),
            ..Default::default()
        }
    }
}

impl Provider for MarketplaceProvider {
    fn id(&self) -> &str {
        "marketplace"
    }

    fn commands(&self) -> Vec<CommandDescriptor> {
        use crate::providers::command::{CommandRoute, CommandSource, ModeKind};
        vec![CommandDescriptor {
            id: "cmd:marketplace".to_string(),
            title: "Browse Extension Marketplace".to_string(),
            chip: "Marketplace".to_string(),
            subtitle: Some("Install extensions".to_string()),
            source: CommandSource::Builtin,
            mode_kind: ModeKind::Scope,
            keywords: vec![
                "marketplace".into(),
                "extensions".into(),
                "install".into(),
                "store".into(),
                "addons".into(),
                "browse".into(),
            ],
            placeholder: Some("Search extensions…".to_string()),
            // Browse scope: entering it lists the whole catalog immediately.
            min_query_len: 0,
            result_kind: "marketplace".to_string(),
            glyph: Some("store".to_string()),
            icon_data_uri: None,
            default_shortcut: None,
            opens_form: false,
            // Browse scope: show the whole catalog, never truncated to max_results.
            uncapped: true,
            route: CommandRoute::Builtin { provider_id: "marketplace".to_string() },
        }]
    }

    // Root search never lists catalog entries - only the command entry above.
    fn search(&self, _query: &str) -> Vec<SearchResult> {
        Vec::new()
    }

    fn search_scoped(&self, _command_id: &str, query: &str) -> Vec<SearchResult> {
        let Some(entries) = self.store.snapshot() else {
            // Never fetched and no disk cache (fresh install, offline).
            return vec![SearchResult {
                id: "market:unavailable".to_string(),
                title: "Marketplace unavailable".to_string(),
                subtitle: Some("Check your connection and try again".to_string()),
                kind: "marketplace-msg".to_string(),
                score: SCORE_MARKETPLACE,
                ..Default::default()
            }];
        };
        let consents = install::load_consents();
        let q = query.trim();

        if q.is_empty() {
            // Browse: updates first, then the catalog alphabetically.
            let mut sorted: Vec<&IndexEntry> = entries.iter().collect();
            sorted.sort_by(|a, b| a.name.cmp(&b.name));
            return sorted
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let mut r = self.row(e, &consents);
                    r.score -= i as f32 * 100.0;
                    r
                })
                .collect();
        }

        let (pattern, mut matcher, mut buf) = fuzzy_setup(q);
        entries
            .iter()
            .filter_map(|e| {
                let keywords = e.keywords.join(" ");
                let fields = [
                    (e.name.as_str(), 1.0),
                    (keywords.as_str(), 1.0),
                    (e.description.as_str(), 0.7),
                ];
                // No quality threshold: the scope is already intent-filtered.
                let (_, score) = fuzzy_best(&pattern, &mut matcher, &mut buf, &fields)?;
                let mut r = self.row(e, &consents);
                r.score += score as f32;
                Some(r)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::marketplace::MarketplaceIndex;

    fn entry(name: &str, version: &str, keywords: &[&str]) -> IndexEntry {
        IndexEntry {
            name: name.to_string(),
            version: version.to_string(),
            api: manifest::SUPPORTED_API,
            description: format!("{name} extension"),
            author: String::new(),
            homepage: String::new(),
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            permissions: install::ConsentPermissions::default(),
            download_url: "https://example.org/pkg.portext".to_string(),
            sha256: "a".repeat(64),
            size_bytes: 1024,
            icon_data_uri: None,
        }
    }

    fn provider(entries: Vec<IndexEntry>) -> MarketplaceProvider {
        MarketplaceProvider::new(Arc::new(Store::with_index(MarketplaceIndex {
            schema: 1,
            extensions: entries,
        })))
    }

    #[test]
    fn browse_lists_all_alphabetically() {
        let p = provider(vec![entry("zeta", "1.0", &[]), entry("alpha", "1.0", &[])]);
        let rows = p.search_scoped("cmd:marketplace", "");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "alpha");
        assert!(rows[0].score > rows[1].score);
        assert_eq!(rows[0].market.as_ref().unwrap().state, "not_installed");
    }

    #[test]
    fn query_filters_by_name_and_keywords() {
        let p = provider(vec![
            entry("emoji", "1.0", &["smiley"]),
            entry("cheatsh", "1.0", &["cheatsheet"]),
        ]);
        let rows = p.search_scoped("cmd:marketplace", "smiley");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "emoji");
    }

    #[test]
    fn empty_store_yields_unavailable_row() {
        let p = MarketplaceProvider::new(Arc::new(Store::empty()));
        let rows = p.search_scoped("cmd:marketplace", "anything");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "marketplace-msg");
    }

    #[test]
    fn incompatible_api_flagged() {
        let mut e = entry("future", "1.0", &[]);
        e.api = manifest::SUPPORTED_API + 1;
        let p = provider(vec![e]);
        let rows = p.search_scoped("cmd:marketplace", "");
        assert_eq!(rows[0].market.as_ref().unwrap().state, "incompatible");
    }
}
