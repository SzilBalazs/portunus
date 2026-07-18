//! Runtime "seen actions" catalog: every action that flows through an
//! extension's results is recorded here (bounded per extension) so the
//! Settings keybinds section can enumerate bindable extension actions without
//! a manifest-side duplicate. Persisted as JSON at
//! `$XDG_DATA_HOME/portunus/ext_actions.json` so the catalog survives
//! restarts and lists actions of extensions that haven't run yet this session.
//!
//! Like the log store, this is a process-global: recording happens deep in
//! the per-keystroke result mapping, where threading a handle through every
//! provider would buy nothing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Cap per extension - an extension emitting unbounded distinct action ids
/// (untrusted input) must not grow the catalog forever.
const MAX_ACTIONS_PER_EXTENSION: usize = 64;
/// Min interval between disk writes. A throttled write is retried by the next
/// change (or a `list_extension_actions` call); a tail lost on quit is benign
/// because the map rebuilds from live traffic.
const SAVE_INTERVAL: Duration = Duration::from_secs(2);

/// One action observed on an extension result, as shown in Settings.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SeenAction {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Extension-shipped default chord, already host-clamped to canonical form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
}

struct Store {
    map: HashMap<String, Vec<SeenAction>>,
    dirty: bool,
    last_save: Option<Instant>,
}

static STORE: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(load()));

fn file_path() -> PathBuf {
    crate::paths::data_dir().join("ext_actions.json")
}

fn load() -> Store {
    let mut map: HashMap<String, Vec<SeenAction>> = std::fs::read_to_string(file_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    // Drop entries for extensions no longer on disk (uninstalled since the
    // last session) and re-enforce the cap on whatever the file claims.
    map.retain(|name, _| super::extensions_dir().join(name).is_dir());
    for actions in map.values_mut() {
        actions.truncate(MAX_ACTIONS_PER_EXTENSION);
    }
    Store { map, dirty: false, last_save: None }
}

/// Records the actions attached to one extension result (already host-clamped
/// by `to_search_result`). Runs on the search mapping path - cheap when
/// nothing changed, which is every call after an action's first sighting.
pub fn record(extension: &str, actions: &[portunus_ext_sdk::Action]) {
    if actions.is_empty() {
        return;
    }
    let mut store = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let list = store.map.entry(extension.to_string()).or_default();
    let mut changed = false;
    for a in actions {
        match list.iter().position(|s| s.id == a.id) {
            Some(i) => {
                let seen = &mut list[i];
                if seen.label != a.label || seen.hint != a.hint || seen.shortcut != a.shortcut {
                    seen.label = a.label.clone();
                    seen.hint = a.hint.clone();
                    seen.shortcut = a.shortcut.clone();
                    changed = true;
                }
            }
            None if list.len() < MAX_ACTIONS_PER_EXTENSION => {
                list.push(SeenAction {
                    id: a.id.clone(),
                    label: a.label.clone(),
                    hint: a.hint.clone(),
                    shortcut: a.shortcut.clone(),
                });
                changed = true;
            }
            None => {}
        }
    }
    if changed {
        store.dirty = true;
    }
    maybe_save(&mut store);
}

fn maybe_save(store: &mut Store) {
    if !store.dirty {
        return;
    }
    if store.last_save.is_some_and(|t| t.elapsed() < SAVE_INTERVAL) {
        return;
    }
    // The throttle stamp advances even on failure so a broken disk can't
    // turn every keystroke into a write attempt.
    store.last_save = Some(Instant::now());
    match serde_json::to_string(&store.map) {
        Ok(json) => match std::fs::write(file_path(), json) {
            Ok(()) => store.dirty = false,
            Err(e) => eprintln!("[extensions] failed to save seen-actions catalog: {e}"),
        },
        Err(e) => eprintln!("[extensions] failed to serialize seen-actions catalog: {e}"),
    }
}

/// The seen-actions catalog, keyed by extension name - backs the Settings
/// keybinds section's extension groups.
#[tauri::command]
pub fn list_extension_actions() -> HashMap<String, Vec<SeenAction>> {
    let mut store = STORE.lock().unwrap_or_else(|e| e.into_inner());
    // Opportunistic flush: opening Settings is a natural persist point, and
    // it guarantees a throttled tail eventually lands on disk.
    store.last_save = None;
    maybe_save(&mut store);
    store.map.clone()
}
