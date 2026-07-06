//! Example Portunus extension: search emoji by name, copy on Enter.
//!
//! Demonstrates a scope command (found by title + keywords, see manifest.toml),
//! structured actions, declarative activate effects (no clipboard permission
//! needed), user settings, and a browse state for the empty query.
//!
//! Build:  cargo build --release --target wasm32-unknown-unknown
//! Install: copy target/.../emoji.wasm to
//!          ~/.local/share/portunus/extensions/emoji/extension.wasm
//!          alongside manifest.toml, then `portunus --reload-extensions`.

// The pdk macros expand to `extism_pdk::…` paths - this alias satisfies them
// without adding extism-pdk as a direct dependency.
use portunus_ext_sdk::guest::extism_pdk;
use portunus_ext_sdk::guest::{plugin_fn, setting_str, FnResult, Json};
use portunus_ext_sdk::{
    Action, ActivateEffect, ActivateInput, ActivateOutput, ExtensionResult, MetadataItem,
    PreviewContent, PreviewInput, ResultIcon, SearchInput, SearchOutput, ToastLevel,
};

/// Pre-encoded result icon (icon.png as base64) - guests embed the encoded
/// form directly rather than pulling in a base64 dependency.
const ICON_B64: &str = include_str!("../icon.b64");

/// (emoji, name, keywords) - a tiny built-in set; a real extension would embed
/// a full emoji database the same way.
const EMOJI: &[(&str, &str, &str)] = &[
    ("😄", "grinning face", "smile happy joy"),
    ("😂", "tears of joy", "laugh lol funny crying"),
    ("❤️", "red heart", "love like"),
    ("👍", "thumbs up", "ok yes approve like"),
    ("🎉", "party popper", "celebrate congrats tada"),
    ("🔥", "fire", "hot lit flame"),
    ("🚀", "rocket", "launch ship fast space"),
    ("😎", "smiling face with sunglasses", "cool"),
    ("🤔", "thinking face", "hmm wonder"),
    ("😭", "loudly crying face", "sad sob tears"),
    ("🙏", "folded hands", "please thanks pray hope"),
    ("💀", "skull", "dead death rip"),
    ("✨", "sparkles", "shiny magic new clean"),
    ("🐛", "bug", "insect error defect"),
    ("🦀", "crab", "rust rustlang"),
];

/// Emoji that take a skin-tone modifier (the `tone` setting).
const TONED: &[&str] = &["thumbs up", "folded hands"];

fn actions() -> Vec<Action> {
    vec![
        Action { id: "copy".into(), label: "Copy emoji".into(), hint: None },
        Action {
            id: "copy-name".into(),
            label: "Copy name".into(),
            hint: Some("as :shortcode: text".into()),
        },
    ]
}

fn to_result(emoji: &str, name: &str, keywords: &str, relevance: f32) -> ExtensionResult {
    ExtensionResult {
        id: name.to_string(),
        title: format!("{emoji} {name}"),
        subtitle: Some(keywords.to_string()),
        relevance,
        actions: actions(),
        icon: Some(ResultIcon {
            mime: "image/png".to_string(),
            data_base64: ICON_B64.trim().to_string(),
        }),
        badge: None,
    }
}

#[plugin_fn]
pub fn search(input: Json<SearchInput>) -> FnResult<Json<SearchOutput>> {
    // `query` is the whole term typed in the scope (e.g. "smi").
    let query = input.0.query.trim().to_lowercase();

    // Empty query = browse state: show the whole set, dataset order.
    if query.is_empty() {
        let results = EMOJI
            .iter()
            .enumerate()
            .map(|(i, (e, n, k))| to_result(e, n, k, 80.0 - i as f32))
            .collect();
        return Ok(Json(SearchOutput { results }));
    }

    let results = EMOJI
        .iter()
        .filter_map(|(emoji, name, keywords)| {
            // Name hits rank above keyword hits; earlier match = more relevant.
            let relevance = if let Some(pos) = name.find(&query) {
                90.0 - pos as f32
            } else if keywords.contains(&query) {
                60.0
            } else {
                return None;
            };
            Some(to_result(emoji, name, keywords, relevance))
        })
        .collect();
    Ok(Json(SearchOutput { results }))
}

#[plugin_fn]
pub fn activate(input: Json<ActivateInput>) -> FnResult<Json<ActivateOutput>> {
    let ActivateInput { result, action, .. } = input.0;
    let effects = match action.as_deref() {
        Some("copy-name") => vec![
            ActivateEffect::CopyText { text: format!(":{}:", result.id.replace(' ', "_")) },
            ActivateEffect::ShowToast { message: format!("Copied :{}:", result.id), level: ToastLevel::Success },
        ],
        // Default (Enter / "copy"): the emoji itself, tone-modified if set.
        _ => {
            let emoji = toned_emoji(&result.id);
            vec![ActivateEffect::CopyText { text: emoji }]
        }
    };
    Ok(Json(ActivateOutput { effects }))
}

/// Applies the user's `tone` setting to emoji that support it.
fn toned_emoji(id: &str) -> String {
    let base = lookup(id).unwrap_or_default().to_string();
    if !TONED.contains(&id) {
        return base;
    }
    let modifier = match setting_str("tone").ok().flatten().as_deref() {
        Some("light") => "\u{1F3FB}",
        Some("medium") => "\u{1F3FD}",
        Some("dark") => "\u{1F3FF}",
        _ => return base,
    };
    format!("{base}{modifier}")
}

#[plugin_fn]
pub fn preview(input: Json<PreviewInput>) -> FnResult<Json<PreviewContent>> {
    let result = input.0.result;
    let Some((emoji, name, keywords)) = entry(&result.id) else {
        return Ok(Json(PreviewContent::Metadata { items: vec![] }));
    };
    Ok(Json(PreviewContent::Metadata {
        items: vec![
            MetadataItem { label: "Emoji".into(), value: emoji.to_string() },
            MetadataItem { label: "Name".into(), value: name.to_string() },
            MetadataItem { label: "Keywords".into(), value: keywords.to_string() },
            MetadataItem { label: "Enter".into(), value: "copy to clipboard".into() },
        ],
    }))
}

fn entry(id: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
    EMOJI.iter().find(|(_, name, _)| *name == id)
}

fn lookup(id: &str) -> Option<&'static str> {
    entry(id).map(|(emoji, _, _)| *emoji)
}
