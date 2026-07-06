//! The preview tier: stream Metadata immediately, then Markdown (README / issue
//! / PR body) once fetched. README link rewriting lives here too.

use crate::api::*;
use crate::render::{fmt_count, humanize, pr_state};
use portunus_ext_sdk::guest::{self, extism_pdk};
use portunus_ext_sdk::guest::{FnResult, Json};
use portunus_ext_sdk::{MetadataItem, PreviewContent};

const README_CAP: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// README link rewriting. The raw README uses paths relative to the repo, which
// the host's markdown preview can't resolve. Rewrite them to absolute URLs:
//   images (md `![](x)`, HTML `src="x"`) -> raw.githubusercontent.com/<full>/<branch>/x
//   links  (md  `[](x)`, HTML `href="x"`) -> github.com/<full>/blob/<branch>/x
// Already-absolute (scheme://, //, data:, mailto:, tel:, #anchor) URLs pass through.
// ---------------------------------------------------------------------------

fn is_absolute_url(u: &str) -> bool {
    let t = u.trim();
    t.is_empty()
        || t.starts_with('#')
        || t.starts_with("//")
        || t.contains("://")
        || t.starts_with("data:")
        || t.starts_with("mailto:")
        || t.starts_with("tel:")
}

fn absolutize(url: &str, base: &str) -> String {
    let u = url.trim();
    let u = u.strip_prefix("./").unwrap_or(u);
    let u = u.trim_start_matches('/');
    format!("{base}/{u}")
}

/// Does the `]` at `rbracket` close an image span (`![...]`) rather than a link?
fn is_image_link(s: &[u8], rbracket: usize) -> bool {
    let mut depth = 0i32;
    let mut j = rbracket;
    while j > 0 {
        j -= 1;
        match s[j] {
            b']' => depth += 1,
            b'[' if depth == 0 => return j > 0 && s[j - 1] == b'!',
            b'[' => depth -= 1,
            _ => {}
        }
    }
    false
}

fn rewrite_attr(input: &str, attr: &str, base: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(pos) = rest.find(attr) {
        out.push_str(&rest[..pos + attr.len()]);
        let after = &rest[pos + attr.len()..];
        let q = after.as_bytes().first().copied();
        if q == Some(b'"') || q == Some(b'\'') {
            let quote = q.unwrap() as char;
            if let Some(end) = after[1..].find(quote) {
                let url = &after[1..1 + end];
                out.push(quote);
                if is_absolute_url(url) { out.push_str(url); } else { out.push_str(&absolutize(url, base)); }
                out.push(quote);
                rest = &after[1 + end + 1..];
                continue;
            }
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

fn rewrite_md_links(input: &str, raw_base: &str, blob_base: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b']' && bytes.get(i + 1) == Some(&b'(') {
            if let Some(rel_end) = input[i + 2..].find(')') {
                let inner = &input[i + 2..i + 2 + rel_end];
                let (url, title) = match inner.find(char::is_whitespace) {
                    Some(sp) => (&inner[..sp], &inner[sp..]),
                    None => (inner, ""),
                };
                let base = if is_image_link(bytes, i) { raw_base } else { blob_base };
                out.push_str("](");
                if is_absolute_url(url) { out.push_str(url); } else { out.push_str(&absolutize(url, base)); }
                out.push_str(title);
                out.push(')');
                i += 2 + rel_end + 1;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn absolutize_readme(readme: &str, full: &str, branch: &str) -> String {
    let raw_base = format!("https://raw.githubusercontent.com/{full}/{branch}");
    let blob_base = format!("https://github.com/{full}/blob/{branch}");
    let s = rewrite_md_links(readme, &raw_base, &blob_base);
    let s = rewrite_attr(&s, "src=", &raw_base);
    rewrite_attr(&s, "href=", &blob_base)
}

fn truncate_md(mut s: String) -> String {
    if s.len() > README_CAP {
        let mut cut = README_CAP;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n\n\u{2026}");
    }
    s
}

// ---------------------------------------------------------------------------
// Degraded / fallback previews
// ---------------------------------------------------------------------------

/// Rate-limited (or otherwise failed) preview: degrade to whatever the repo
/// cache knows plus an actionable hint, never a bare error.
fn degraded_preview(full: Option<&str>, status: u16) -> PreviewContent {
    let mut items = Vec::new();
    if let Some(full) = full {
        if let Some(r) = cache_read::<CachedRepo>(CACHE_REPOS).into_iter().find(|r| r.full_name == full) {
            items.push(MetadataItem { label: "Stars".into(), value: fmt_count(r.stars) });
            if let Some(l) = &r.language {
                items.push(MetadataItem { label: "Language".into(), value: l.clone() });
            }
            if let Some(d) = r.description.as_deref().filter(|d| !d.is_empty()) {
                items.push(MetadataItem { label: "About".into(), value: d.to_string() });
            }
        }
    }
    if is_rate_limited(status) {
        let hint = if token().is_some() {
            "GitHub rate limit exhausted - try again in a few minutes".to_string()
        } else {
            "GitHub rate limit - add a token in extension settings for 5000 req/h".to_string()
        };
        items.push(MetadataItem { label: "Rate limited".into(), value: hint });
    } else {
        items.push(MetadataItem { label: "Error".into(), value: format!("HTTP {status}") });
    }
    PreviewContent::Metadata { items }
}

// ---------------------------------------------------------------------------
// Entry point: dispatch on the result id prefix.
// ---------------------------------------------------------------------------

pub fn preview(id: &str) -> FnResult<Json<PreviewContent>> {
    if let Some(full) = id.strip_prefix("repo:") {
        return preview_repo(full);
    }
    if let Some((repo, number)) = id.strip_prefix("pr:").and_then(|s| s.split_once('#')) {
        return preview_pr(repo, number);
    }
    if let Some((repo, number)) = id.strip_prefix("issue:").and_then(|s| s.split_once('#')) {
        return preview_issue(repo, number);
    }
    if let Some(login) = id.strip_prefix("user:") {
        return preview_user(login);
    }
    if let Some(rest) = id.strip_prefix("notif|") {
        return Ok(Json(preview_notif(rest)));
    }
    Ok(Json(PreviewContent::Metadata {
        items: vec![MetadataItem {
            label: "GitHub".into(),
            value: "select a result to preview it".into(),
        }],
    }))
}

fn preview_repo(full: &str) -> FnResult<Json<PreviewContent>> {
    // Instant first paint from the kv cache (no network).
    if let Some(r) = cache_read::<CachedRepo>(CACHE_REPOS).into_iter().find(|r| r.full_name == full) {
        let cached = PreviewContent::Metadata {
            items: vec![
                MetadataItem { label: "Stars".into(), value: fmt_count(r.stars) },
                MetadataItem { label: "Language".into(), value: r.language.clone().unwrap_or_else(|| "-".into()) },
                MetadataItem { label: "About".into(), value: r.description.clone().unwrap_or_default() },
            ],
        };
        if !guest::emit_preview_update(&cached).unwrap_or(false) {
            return Ok(Json(cached));
        }
    }

    let (status, body) = api_get(&format!("/repos/{full}"), ACCEPT_JSON)?;
    if status != 200 {
        return Ok(Json(degraded_preview(Some(full), status)));
    }
    let d: RepoDetail = serde_json::from_str(&body).map_err(extism_pdk::Error::from)?;

    let mut items = vec![
        MetadataItem { label: "Stars".into(), value: fmt_count(d.stargazers_count) },
        MetadataItem { label: "Forks".into(), value: fmt_count(d.forks_count) },
        MetadataItem { label: "Open issues".into(), value: fmt_count(d.open_issues_count) },
    ];
    if let Some(l) = &d.language {
        items.push(MetadataItem { label: "Language".into(), value: l.clone() });
    }
    if let Some(l) = &d.license {
        let name = l.spdx_id.clone().filter(|s| s != "NOASSERTION").unwrap_or_else(|| l.name.clone());
        items.push(MetadataItem { label: "License".into(), value: name });
    }
    if !d.topics.is_empty() {
        items.push(MetadataItem { label: "Topics".into(), value: d.topics.join(", ") });
    }
    if let Some(h) = d.homepage.as_deref().filter(|h| !h.trim().is_empty()) {
        items.push(MetadataItem { label: "Homepage".into(), value: h.to_string() });
    }
    items.push(MetadataItem { label: "Default branch".into(), value: d.default_branch.clone() });
    if let Some(ts) = &d.pushed_at {
        items.push(MetadataItem { label: "Last push".into(), value: humanize(ts) });
    }
    if let Some(desc) = d.description.as_deref().filter(|s| !s.is_empty()) {
        items.push(MetadataItem { label: "About".into(), value: desc.to_string() });
    }
    let metadata = PreviewContent::Metadata { items };

    if !guest::emit_preview_update(&metadata).unwrap_or(false) {
        return Ok(Json(metadata));
    }

    match api_get(&format!("/repos/{full}/readme"), "application/vnd.github.raw+json") {
        Ok((200, readme)) if !readme.trim().is_empty() => {
            let readme = truncate_md(absolutize_readme(&readme, full, &d.default_branch));
            Ok(Json(PreviewContent::Markdown { content: format!("# {}\n\n{readme}", d.full_name) }))
        }
        _ => Ok(Json(metadata)),
    }
}

fn preview_issue(repo: &str, number: &str) -> FnResult<Json<PreviewContent>> {
    let (status, body) = api_get(&format!("/repos/{repo}/issues/{number}"), ACCEPT_JSON)?;
    if status != 200 {
        return Ok(Json(degraded_preview(None, status)));
    }
    let d: IssueDetail = serde_json::from_str(&body).map_err(extism_pdk::Error::from)?;

    let mut items = vec![
        MetadataItem { label: "State".into(), value: d.state.clone() },
        MetadataItem { label: "Comments".into(), value: fmt_count(d.comments) },
    ];
    if let Some(u) = &d.user {
        items.push(MetadataItem { label: "Author".into(), value: u.login.clone() });
    }
    if !d.assignees.is_empty() {
        let names: Vec<&str> = d.assignees.iter().map(|a| a.login.as_str()).collect();
        items.push(MetadataItem { label: "Assignees".into(), value: names.join(", ") });
    }
    if !d.labels.is_empty() {
        let names: Vec<&str> = d.labels.iter().map(|l| l.name.as_str()).collect();
        items.push(MetadataItem { label: "Labels".into(), value: names.join(", ") });
    }
    if let Some(ts) = &d.created_at {
        items.push(MetadataItem { label: "Opened".into(), value: humanize(ts) });
    }
    if let Some(ts) = &d.updated_at {
        items.push(MetadataItem { label: "Updated".into(), value: humanize(ts) });
    }
    let metadata = PreviewContent::Metadata { items };

    let body_md = d.body.as_deref().unwrap_or("").trim();
    if body_md.is_empty() || !guest::emit_preview_update(&metadata).unwrap_or(false) {
        return Ok(Json(metadata));
    }
    Ok(Json(PreviewContent::Markdown {
        content: truncate_md(format!("# {} ({})\n\n{}", d.title, d.state, body_md)),
    }))
}

fn preview_pr(repo: &str, number: &str) -> FnResult<Json<PreviewContent>> {
    let (status, body) = api_get(&format!("/repos/{repo}/pulls/{number}"), ACCEPT_JSON)?;
    if status != 200 {
        return Ok(Json(degraded_preview(None, status)));
    }
    let d: PrDetail = serde_json::from_str(&body).map_err(extism_pdk::Error::from)?;

    let state = pr_state(d.state == "open", d.merged, d.draft);
    let mut items = vec![
        MetadataItem { label: "State".into(), value: state.into() },
        MetadataItem {
            label: "Diff".into(),
            value: format!("+{} \u{2212}{} in {} file{}", d.additions, d.deletions, d.changed_files, if d.changed_files == 1 { "" } else { "s" }),
        },
        MetadataItem { label: "Commits".into(), value: fmt_count(d.commits) },
        MetadataItem { label: "Comments".into(), value: fmt_count(d.comments) },
    ];
    if let (Some(base), Some(head)) = (&d.base, &d.head) {
        items.push(MetadataItem { label: "Branch".into(), value: format!("{} \u{2190} {}", base.name, head.name) });
    }
    if state == "open" {
        if let Some(m) = d.mergeable_state.as_deref().filter(|m| !m.is_empty() && *m != "unknown") {
            items.push(MetadataItem { label: "Mergeable".into(), value: m.to_string() });
        }
    }
    if let Some(u) = &d.user {
        items.push(MetadataItem { label: "Author".into(), value: u.login.clone() });
    }
    if !d.labels.is_empty() {
        let names: Vec<&str> = d.labels.iter().map(|l| l.name.as_str()).collect();
        items.push(MetadataItem { label: "Labels".into(), value: names.join(", ") });
    }
    if let Some(ts) = &d.created_at {
        items.push(MetadataItem { label: "Opened".into(), value: humanize(ts) });
    }
    if let Some(ts) = &d.updated_at {
        items.push(MetadataItem { label: "Updated".into(), value: humanize(ts) });
    }
    let metadata = PreviewContent::Metadata { items };

    let body_md = d.body.as_deref().unwrap_or("").trim();
    if body_md.is_empty() || !guest::emit_preview_update(&metadata).unwrap_or(false) {
        return Ok(Json(metadata));
    }
    Ok(Json(PreviewContent::Markdown {
        content: truncate_md(format!("# {} ({state})\n\n{}", d.title, body_md)),
    }))
}

fn preview_user(login: &str) -> FnResult<Json<PreviewContent>> {
    let (status, body) = api_get(&format!("/users/{login}"), ACCEPT_JSON)?;
    if status != 200 {
        return Ok(Json(degraded_preview(None, status)));
    }
    let d: UserDetail = serde_json::from_str(&body).map_err(extism_pdk::Error::from)?;

    let kind = if d.kind.eq_ignore_ascii_case("organization") { "Organization" } else { "User" };
    let mut items = vec![
        MetadataItem { label: "Login".into(), value: d.login.clone() },
        MetadataItem { label: "Type".into(), value: kind.into() },
        MetadataItem { label: "Followers".into(), value: fmt_count(d.followers) },
        MetadataItem { label: "Following".into(), value: fmt_count(d.following) },
        MetadataItem { label: "Public repos".into(), value: fmt_count(d.public_repos) },
    ];
    if let Some(n) = d.name.as_deref().filter(|s| !s.is_empty()) {
        items.push(MetadataItem { label: "Name".into(), value: n.to_string() });
    }
    if let Some(c) = d.company.as_deref().filter(|s| !s.is_empty()) {
        items.push(MetadataItem { label: "Company".into(), value: c.to_string() });
    }
    if let Some(l) = d.location.as_deref().filter(|s| !s.is_empty()) {
        items.push(MetadataItem { label: "Location".into(), value: l.to_string() });
    }
    if let Some(b) = d.blog.as_deref().filter(|s| !s.trim().is_empty()) {
        items.push(MetadataItem { label: "Website".into(), value: b.to_string() });
    }
    if let Some(bio) = d.bio.as_deref().filter(|s| !s.is_empty()) {
        items.push(MetadataItem { label: "Bio".into(), value: bio.to_string() });
    }
    Ok(Json(PreviewContent::Metadata { items }))
}

/// Notification preview is offline: everything needed was packed into the id
/// (`url|type|reason|repo`), so no network round-trip on selection.
fn preview_notif(rest: &str) -> PreviewContent {
    let parts: Vec<&str> = rest.split('|').collect();
    let get = |i: usize| parts.get(i).copied().unwrap_or("");
    PreviewContent::Metadata {
        items: vec![
            MetadataItem { label: "Type".into(), value: get(1).to_string() },
            MetadataItem { label: "Reason".into(), value: get(2).replace('_', " ") },
            MetadataItem { label: "Repository".into(), value: get(3).to_string() },
            MetadataItem { label: "Link".into(), value: get(0).to_string() },
        ],
    }
}
