//! Example Portunus extension: search emoji by name, copy on Enter.
//!
//! Build:  cargo build --release --target wasm32-unknown-unknown
//! Install: copy target/.../emoji.wasm to
//!          ~/.local/share/portunus/extensions/emoji/extension.wasm
//!          alongside manifest.toml, then `portunus --reload-extensions`.

// The pdk macros expand to `extism_pdk::…` paths - this alias satisfies them
// without adding extism-pdk as a direct dependency.
use portunus_ext_sdk::guest::extism_pdk;
use portunus_ext_sdk::guest::{clipboard, plugin_fn, FnResult, Json};
use portunus_ext_sdk::{
    ActivateInput, ActivateOutput, ExtensionResult, MetadataItem, PreviewContent, PreviewInput,
    ResultIcon, SearchInput, SearchOutput,
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

#[plugin_fn]
pub fn search(input: Json<SearchInput>) -> FnResult<Json<SearchOutput>> {
    let query = input.0.query.trim().to_lowercase();
    if query.is_empty() {
        return Ok(Json(SearchOutput::default()));
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
            Some(ExtensionResult {
                id: name.to_string(),
                title: format!("{emoji} {name}"),
                subtitle: Some(keywords.to_string()),
                relevance,
                actions: vec!["copy".to_string()],
                icon: Some(ResultIcon {
                    mime: "image/png".to_string(),
                    data_base64: ICON_B64.trim().to_string(),
                }),
            })
        })
        .collect();
    Ok(Json(SearchOutput { results }))
}

#[plugin_fn]
pub fn activate(input: Json<ActivateInput>) -> FnResult<Json<ActivateOutput>> {
    let result = input.0.result;
    let emoji = lookup(&result.id).unwrap_or_default();
    clipboard(emoji)?;
    Ok(Json(ActivateOutput { ok: true }))
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
