//! Example Portunus extension: cheat.sh lookups. Demonstrates a scope command
//! (found by title + keywords; the extension sees the whole term), a
//! `[background]` refresh that warms the kv cache, an async `query` export
//! that streams the live sheet in over the instant kv results, structured
//! actions, and declarative activate effects (no clipboard/open_url perms).

use portunus_ext_sdk::guest::extism_pdk::{self, http, HttpRequest};
use portunus_ext_sdk::guest::{self, kv_read, kv_write, plugin_fn, FnResult, Json};
use portunus_ext_sdk::{
    Action, ActivateEffect, ActivateInput, ActivateOutput, ExtensionResult, PreviewContent,
    PreviewInput, QueryInput, QueryOutput, RefreshInput, RefreshOutput, SearchInput, SearchOutput,
    ToastLevel,
};

fn fetch_text(url: &str) -> Result<String, extism_pdk::Error> {
    let mut req = HttpRequest::new(url);
    // Without a curl-like User-Agent cheat.sh returns HTML instead of plain text.
    req.headers.insert("User-Agent".into(), "curl/7.86.0".into());
    let resp = http::request::<Vec<u8>>(&req, None)?;
    let raw = String::from_utf8_lossy(&resp.body()).into_owned();
    Ok(strip_ansi(&raw))
}

/// Strip ANSI escape sequences (e.g. \x1b[38;5;248m) from a string.
/// cheat.sh ignores ?T for some pages and still returns coloured output.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            // consume digits, semicolons, then the terminating letter
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

// Bucket key for a term: "b_<first_char>" so search reads only a small slice.
fn bucket_key(term: &str) -> Option<String> {
    term.chars().next().map(|c| format!("b_{c}"))
}

// Fetch cheat.sh/:list, strip RFC/meta/blank entries, store one kv key per
// first character. Sets sentinel key "cached" when done.
fn ensure_list_cached() -> Result<(), extism_pdk::Error> {
    if kv_read("cached")?.is_some() {
        return Ok(());
    }
    let raw = fetch_text("https://cheat.sh/:list")?;

    let mut buckets: std::collections::HashMap<char, Vec<&str>> = Default::default();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("rfc") || line.starts_with(':') {
            continue;
        }
        if let Some(c) = line.chars().next() {
            buckets.entry(c).or_default().push(line);
        }
    }
    for (c, terms) in &buckets {
        kv_write(&format!("b_{c}"), &terms.join("\n"))?;
    }
    kv_write("cached", "1")
}

fn actions() -> Vec<Action> {
    vec![
        Action { id: "open".into(), label: "Open cheat sheet".into(), hint: Some("in browser".into()), opens_form: false },
        Action { id: "copy".into(), label: "Copy sheet text".into(), hint: Some("fetches plain text".into()), opens_form: false },
    ]
}

/// Background refresh (load + daily): warm the :list cache so search can
/// prefix-match instantly. Runs on a dedicated instance, off the keystroke path.
#[plugin_fn]
pub fn refresh(_input: Json<RefreshInput>) -> FnResult<Json<RefreshOutput>> {
    ensure_list_cached()?;
    Ok(Json(RefreshOutput::default()))
}

/// Async tier: confirm the exact term has a live cheat sheet and stream a
/// "live" result in. Runs on a dedicated instance with a generous budget while
/// the instant kv-backed `search` results are already on screen; emitting the
/// same id as the exact-match search row replaces it in place.
#[plugin_fn]
pub fn query(input: Json<QueryInput>) -> FnResult<Json<QueryOutput>> {
    let term = input.0.query.trim().to_lowercase();
    if term.is_empty() {
        return Ok(Json(QueryOutput::default()));
    }
    // Blocking network is fine here - it's off the keystroke path.
    let sheet = match fetch_text(&format!("https://cheat.sh/{term}?T")) {
        Ok(s) => s,
        Err(_) => return Ok(Json(QueryOutput::default())),
    };
    // cheat.sh answers a missing term with an "Unknown topic." page. Don't
    // enrich the row with that - leave the cached search row as-is.
    if sheet.trim_start().starts_with("Unknown topic") {
        return Ok(Json(QueryOutput::default()));
    }
    // First non-empty, non-comment line = a one-line preview of the sheet.
    let snippet = sheet
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("cheat sheet")
        .to_string();

    let result = ExtensionResult {
        id: term.clone(), // same id as the exact-match search row -> replaces it
        title: format!("cheat.sh/{term}"),
        subtitle: Some(snippet),
        relevance: 100.0,
        actions: actions(),
        // Distinguishes this freshly-fetched, network-confirmed row from the
        // instant cached prefix matches around it.
        badge: Some("online".into()),
        ..Default::default()
    };
    // Push it as a partial batch; if cancelled (new keystroke) `emit` is false.
    let _ = guest::emit(vec![result])?;
    Ok(Json(QueryOutput::default()))
}

#[plugin_fn]
pub fn search(input: Json<SearchInput>) -> FnResult<Json<SearchOutput>> {
    // `query` is the whole term typed in the scope (e.g. "tar").
    let term_owned = input.0.query.trim().to_lowercase();
    let term = term_owned.as_str();
    if term.is_empty() {
        return Ok(Json(SearchOutput::default()));
    }

    if kv_read("cached")?.is_none() {
        return Ok(Json(SearchOutput {
            results: vec![ExtensionResult {
                id: term.to_string(),
                title: format!("cheat.sh/{term}"),
                subtitle: Some("list not cached yet - refreshing in background".into()),
                relevance: 50.0,
                actions: actions(),
                badge: Some("uncached".into()),
                ..Default::default()
            }],
        }));
    }

    // Read only the bucket for the term's first character.
    let Some(key) = bucket_key(term) else {
        return Ok(Json(SearchOutput::default()));
    };
    let Some(bucket) = kv_read(&key)? else {
        return Ok(Json(SearchOutput::default()));
    };

    let mut results: Vec<ExtensionResult> = bucket
        .lines()
        .filter(|entry| entry.starts_with(term))
        .map(|entry| {
            let relevance = 100.0 * (term.len() as f32 / entry.len() as f32);
            ExtensionResult {
                id: entry.to_string(),
                title: format!("cheat.sh/{entry}"),
                subtitle: Some("Cheat sheet".into()),
                relevance,
                actions: actions(),
                ..Default::default()
            }
        })
        .collect();

    results.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(20);

    Ok(Json(SearchOutput { results }))
}

#[plugin_fn]
pub fn activate(input: Json<ActivateInput>) -> FnResult<Json<ActivateOutput>> {
    let id = input.0.result.id;
    let action = input.0.action.as_deref().unwrap_or("open");

    // Populate list cache on first activate so subsequent searches have
    // prefix-matching available even before the background refresh lands.
    let _ = ensure_list_cached();

    let effects = match action {
        "copy" => {
            let body = fetch_text(&format!("https://cheat.sh/{id}?T"))?;
            vec![
                ActivateEffect::CopyText { text: body },
                ActivateEffect::ShowToast { message: format!("Copied cheat.sh/{id}"), level: ToastLevel::Success },
            ]
        }
        _ => vec![ActivateEffect::OpenUrl { url: format!("https://cheat.sh/{id}") }],
    };

    Ok(Json(ActivateOutput { effects }))
}

#[plugin_fn]
pub fn preview(input: Json<PreviewInput>) -> FnResult<Json<PreviewContent>> {
    let id = input.0.result.id;
    let body = match fetch_text(&format!("https://cheat.sh/{id}?T")) {
        Ok(b) => b,
        Err(_) => return Ok(Json(PreviewContent::Metadata { items: vec![] })),
    };
    Ok(Json(parse_cheatsh(&body)))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Parse cheat.sh plain-text output into an `Html` preview using host utility classes.
///
/// Strips the `#[cheat:*]` header and `---`…`---` YAML frontmatter, groups
/// lines into sections by `# Heading` markers, and renders each section as a
/// command/description layout. Falls back to a `Code` block if no structure found.
fn parse_cheatsh(raw: &str) -> PreviewContent {
    let mut lines = raw.lines().peekable();

    // Skip `#[cheat:*]` / `#[cheat.sheets:*]` header line.
    if lines.peek().map(|l| l.starts_with("#[cheat")).unwrap_or(false) {
        lines.next();
    }

    // Skip `---` … `---` YAML frontmatter block if present.
    if lines.peek().map(|l| l.trim() == "---").unwrap_or(false) {
        lines.next();
        for line in lines.by_ref() {
            if line.trim() == "---" {
                break;
            }
        }
    }

    // heading → rows
    let mut sections: Vec<(Option<String>, Vec<Vec<String>>)> = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_rows: Vec<Vec<String>> = Vec::new();

    for line in lines {
        if line.starts_with("#[") {
            continue;
        }
        if let Some(heading) = line.strip_prefix("# ") {
            if !current_rows.is_empty() {
                sections.push((current_heading.take(), current_rows));
                current_rows = Vec::new();
            }
            current_heading = Some(heading.trim().to_string());
        } else if line.trim().is_empty() {
            // skip
        } else {
            current_rows.push(split_row(line));
        }
    }
    if !current_rows.is_empty() {
        sections.push((current_heading, current_rows));
    }

    if sections.is_empty() {
        return PreviewContent::Code { lang: "text".into(), content: raw.trim().to_string() };
    }

    let mut html = String::from(
        r#"<div class="col" style="gap:0;padding:14px 18px;min-height:100%">"#,
    );
    for (i, (heading, rows)) in sections.iter().enumerate() {
        if i > 0 {
            html.push_str(r#"<hr class="divider">"#);
        }
        html.push_str(r#"<div style="padding:8px 0">"#);
        if let Some(h) = heading {
            html.push_str(&format!(
                r#"<div class="text-label" style="margin-bottom:6px">{}</div>"#,
                html_escape(h)
            ));
        }
        html.push_str(r#"<div class="col" style="gap:3px">"#);
        for row in rows {
            match row.as_slice() {
                [] => {}
                [solo] => {
                    html.push_str(&format!(
                        r#"<code class="mono accent-line" style="display:block;padding-top:2px;padding-bottom:2px">{}</code>"#,
                        html_escape(solo)
                    ));
                }
                [cmd, rest @ ..] => {
                    html.push_str(&format!(
                        r#"<div class="row" style="align-items:baseline;gap:0"><code class="mono" style="color:var(--accent);white-space:nowrap;padding-right:12px;flex-shrink:0">{}</code><span class="text-dim" style="font-size:12px">{}</span></div>"#,
                        html_escape(cmd),
                        html_escape(&rest.join("  "))
                    ));
                }
            }
        }
        html.push_str("</div></div>");
    }
    html.push_str("</div>");

    PreviewContent::Html { content: html }
}

/// Split a cheat.sh row on the first run of ≥2 spaces.
/// Returns `[cmd, desc]` or `[whole_line]` if no split point found.
fn split_row(line: &str) -> Vec<String> {
    // Find the byte index of the first occurrence of two or more spaces.
    let mut chars = line.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == ' ' {
            if chars.peek().map(|(_, nc)| *nc == ' ').unwrap_or(false) {
                let cmd = line[..i].trim();
                let desc = line[i..].trim();
                if !cmd.is_empty() && !desc.is_empty() {
                    return vec![cmd.to_string(), desc.to_string()];
                }
            }
        }
    }
    vec![line.trim().to_string()]
}
