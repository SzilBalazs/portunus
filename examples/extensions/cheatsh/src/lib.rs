use portunus_ext_sdk::guest::extism_pdk::{self, http, HttpRequest};
use portunus_ext_sdk::guest::{clipboard, kv_read, kv_write, open, plugin_fn, FnResult, Json};
use portunus_ext_sdk::{
    ActivateInput, ActivateOutput, ExtensionResult, PreviewContent, PreviewInput, SearchInput,
    SearchOutput,
};

// Valid trigger prefixes: ≥2-char prefixes of "cheat.sh" or "ch.sh",
// followed by a space or "/" then the term.
const PREFIXES: &[&str] = &[
    "cheat.sh", "cheat.s", "cheat.", "cheat", "ch.sh", "ch.s", "ch.", "chea", "che", "ch",
];

fn parse_term(query: &str) -> Option<&str> {
    for prefix in PREFIXES {
        if let Some(rest) = query.strip_prefix(prefix) {
            if let Some(term) = rest.strip_prefix('/').or_else(|| rest.strip_prefix(' ')) {
                let t = term.trim();
                if !t.is_empty() {
                    return Some(t);
                }
            }
        }
    }
    None
}

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

#[plugin_fn]
pub fn search(input: Json<SearchInput>) -> FnResult<Json<SearchOutput>> {
    let query = input.0.query.trim().to_lowercase();
    let Some(term) = parse_term(&query) else {
        return Ok(Json(SearchOutput::default()));
    };

    if kv_read("cached")?.is_none() {
        return Ok(Json(SearchOutput {
            results: vec![ExtensionResult {
                id: term.to_string(),
                title: format!("cheat.sh/{term}"),
                subtitle: Some("list not cached yet, press Enter to load".into()),
                relevance: 50.0,
                actions: vec!["open".into()],
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
                relevance,
                actions: vec!["open".into(), "copy".into()],
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
    // prefix-matching available.
    let _ = ensure_list_cached();

    match action {
        "copy" => {
            let body = fetch_text(&format!("https://cheat.sh/{id}?T"))?;
            clipboard(&body)?;
        }
        _ => {
            open(&format!("https://cheat.sh/{id}"))?;
        }
    }

    Ok(Json(ActivateOutput { ok: true }))
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
