//! Example Portunus extension: live GitHub search. The v3 breadth showcase:
//! a secret `token` setting (stored in the system keyring), an async `query`
//! export that hits /search/repositories and /search/issues and streams each
//! endpoint's batch as it returns, a kv repo cache that `search` serves
//! instantly (same ids, so live rows swap cached rows in place), streaming
//! Metadata -> Markdown previews via `emit_preview_update`, declarative
//! open/copy activate effects, and a daily `[background]` refresh that warms
//! the "my repos" cache.

use portunus_ext_sdk::guest::extism_pdk::{self, http, HttpRequest};
use portunus_ext_sdk::guest::{self, kv_read, kv_write, plugin_fn, FnResult, Json};
use portunus_ext_sdk::{
    Action, ActivateEffect, ActivateInput, ActivateOutput, ExtensionResult, MetadataItem,
    PreviewContent, PreviewInput, QueryInput, QueryOutput, RefreshInput, RefreshOutput,
    ResultIcon, SearchInput, SearchOutput,
};
use serde::{Deserialize, Serialize};

const API: &str = "https://api.github.com";
const CACHE_KEY: &str = "repos";
const CACHE_CAP: usize = 200;
const README_CAP: usize = 64 * 1024;

// Octicon glyphs (base64 PNG) shown next to each result row. Colored to match
// GitHub's semantics: repo blue, pull-request purple, issue green.
const ICON_REPO_B64: &str = include_str!("../icon_repo.b64");
const ICON_PR_B64: &str = include_str!("../icon_pr.b64");
const ICON_ISSUE_B64: &str = include_str!("../icon_issue.b64");

fn png_icon(b64: &str) -> ResultIcon {
    ResultIcon { mime: "image/png".into(), data_base64: b64.trim().to_string() }
}

// ---------------------------------------------------------------------------
// GitHub API plumbing
// ---------------------------------------------------------------------------

fn token() -> Option<String> {
    guest::setting_str("token")
        .ok()
        .flatten()
        .filter(|t| !t.trim().is_empty())
}

/// GET an api.github.com path. Returns (status, body). GitHub rejects
/// requests without a User-Agent; the Authorization header is attached only
/// when the user has stored a token (the extension works without one, at
/// unauthenticated rate limits and public-only visibility).
fn api_get(path: &str, accept: &str) -> Result<(u16, String), extism_pdk::Error> {
    let mut req = HttpRequest::new(format!("{API}{path}"));
    req.headers.insert("User-Agent".into(), "portunus-gh-ext".into());
    req.headers.insert("Accept".into(), accept.into());
    req.headers.insert("X-GitHub-Api-Version".into(), "2022-11-28".into());
    if let Some(t) = token() {
        req.headers.insert("Authorization".into(), format!("Bearer {t}"));
    }
    let resp = http::request::<Vec<u8>>(&req, None)?;
    let body = String::from_utf8_lossy(&resp.body()).into_owned();
    Ok((resp.status_code(), body))
}

fn is_rate_limited(status: u16) -> bool {
    status == 403 || status == 429
}

/// Percent-encode a search query for use inside `?q=`.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// Deserialization targets for the handful of fields each endpoint needs.

#[derive(Deserialize)]
struct SearchResp<T> {
    items: Vec<T>,
}

#[derive(Deserialize)]
struct RepoItem {
    full_name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    stargazers_count: u64,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    private: bool,
}

#[derive(Deserialize)]
struct IssueItem {
    title: String,
    number: u64,
    state: String,
    #[serde(default)]
    comments: u64,
    repository_url: String,
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct RepoDetail {
    full_name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    stargazers_count: u64,
    #[serde(default)]
    forks_count: u64,
    #[serde(default)]
    open_issues_count: u64,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    license: Option<License>,
    #[serde(default)]
    default_branch: String,
    #[serde(default)]
    pushed_at: Option<String>,
}

#[derive(Deserialize)]
struct License {
    #[serde(default)]
    spdx_id: Option<String>,
    name: String,
}

#[derive(Deserialize)]
struct IssueDetail {
    title: String,
    state: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    comments: u64,
    #[serde(default)]
    user: Option<User>,
    #[serde(default)]
    labels: Vec<Label>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

#[derive(Deserialize)]
struct User {
    login: String,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

// ---------------------------------------------------------------------------
// kv repo cache: the instant tier's data source
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct CachedRepo {
    full_name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    stars: u64,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    private: bool,
}

impl From<&RepoItem> for CachedRepo {
    fn from(r: &RepoItem) -> Self {
        CachedRepo {
            full_name: r.full_name.clone(),
            description: r.description.clone(),
            stars: r.stargazers_count,
            language: r.language.clone(),
            private: r.private,
        }
    }
}

fn read_cache() -> Vec<CachedRepo> {
    kv_read(CACHE_KEY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_cache(repos: &[CachedRepo]) {
    if let Ok(json) = serde_json::to_string(repos) {
        let _ = kv_write(CACHE_KEY, &json);
    }
}

/// Prepend `fresh` onto the cache, dedupe by full_name, cap the size. Keeps
/// searched-for repos servable instantly on the next keystroke.
fn merge_cache(fresh: &[CachedRepo]) {
    let mut merged: Vec<CachedRepo> = fresh.to_vec();
    for old in read_cache() {
        if !merged.iter().any(|r| r.full_name == old.full_name) {
            merged.push(old);
        }
    }
    merged.truncate(CACHE_CAP);
    write_cache(&merged);
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn fmt_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_949 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

/// Parse an ISO 8601 timestamp ("2024-05-01T12:30:00Z") into unix seconds.
fn parse_iso(ts: &str) -> Option<i64> {
    let date = ts.get(0..10)?;
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    let time = ts.get(11..19)?;
    let mut parts = time.split(':');
    let hh: i64 = parts.next()?.parse().ok()?;
    let mm: i64 = parts.next()?.parse().ok()?;
    let ss: i64 = parts.next()?.parse().ok()?;
    // Days since epoch via the civil-from-days inverse (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3_600 + mm * 60 + ss)
}

/// "3 days ago" from an ISO timestamp, using the host clock.
fn humanize(ts: &str) -> String {
    let Some(then) = parse_iso(ts) else {
        return ts.to_string();
    };
    let now = guest::now().map(|ms| (ms / 1_000) as i64).unwrap_or(then);
    let secs = (now - then).max(0);
    let (n, unit) = match secs {
        0..=59 => return "just now".into(),
        60..=3_599 => (secs / 60, "minute"),
        3_600..=86_399 => (secs / 3_600, "hour"),
        86_400..=2_591_999 => (secs / 86_400, "day"),
        2_592_000..=31_535_999 => (secs / 2_592_000, "month"),
        _ => (secs / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

fn repo_subtitle(stars: u64, language: Option<&str>, description: Option<&str>) -> String {
    let mut parts = vec![format!("\u{2605} {}", fmt_count(stars))];
    if let Some(l) = language {
        parts.push(l.to_string());
    }
    if let Some(d) = description.filter(|d| !d.is_empty()) {
        parts.push(d.to_string());
    }
    parts.join(" \u{b7} ")
}

// ---------------------------------------------------------------------------
// Results and actions
// ---------------------------------------------------------------------------

fn repo_actions() -> Vec<Action> {
    vec![
        Action { id: "open".into(), label: "Open on GitHub".into(), hint: Some("in browser".into()) },
        Action { id: "copy-clone".into(), label: "Copy clone URL".into(), hint: Some("https".into()) },
        Action { id: "copy-gh".into(), label: "Copy gh clone command".into(), hint: Some("gh repo clone".into()) },
    ]
}

fn issue_actions() -> Vec<Action> {
    vec![
        Action { id: "open".into(), label: "Open on GitHub".into(), hint: Some("in browser".into()) },
        Action { id: "copy-url".into(), label: "Copy URL".into(), hint: None },
    ]
}

fn repo_result(r: &CachedRepo, relevance: f32, badge: &str) -> ExtensionResult {
    ExtensionResult {
        id: format!("repo:{}", r.full_name),
        title: r.full_name.clone(),
        subtitle: Some(repo_subtitle(r.stars, r.language.as_deref(), r.description.as_deref())),
        relevance,
        actions: repo_actions(),
        icon: Some(png_icon(ICON_REPO_B64)),
        badge: Some(if r.private { "private".into() } else { badge.into() }),
        ..Default::default()
    }
}

/// Repo full name ("owner/name") from an API url like
/// "https://api.github.com/repos/owner/name".
fn repo_from_api_url(url: &str) -> Option<&str> {
    url.split_once("/repos/").map(|(_, full)| full)
}

fn issue_result(i: &IssueItem, relevance: f32) -> Option<ExtensionResult> {
    let repo = repo_from_api_url(&i.repository_url)?;
    let is_pr = i.pull_request.is_some();
    let kind = if is_pr { "PR" } else { "issue" };
    Some(ExtensionResult {
        id: format!("issue:{repo}#{}", i.number),
        title: i.title.clone(),
        subtitle: Some(format!(
            "{} {kind} \u{b7} {repo}#{} \u{b7} {} comments",
            i.state,
            i.number,
            fmt_count(i.comments)
        )),
        relevance,
        actions: issue_actions(),
        icon: Some(png_icon(if is_pr { ICON_PR_B64 } else { ICON_ISSUE_B64 })),
        badge: Some("live".into()),
        ..Default::default()
    })
}

fn rate_limit_result() -> ExtensionResult {
    let hint = if token().is_some() {
        "rate limit exhausted - try again in a minute"
    } else {
        "add a GitHub token in extension settings for higher limits"
    };
    ExtensionResult {
        id: "gh:ratelimit".into(),
        title: "GitHub rate limit hit".into(),
        subtitle: Some(hint.into()),
        relevance: 1.0,
        actions: vec![Action {
            id: "open".into(),
            label: "Open token settings".into(),
            hint: Some("github.com/settings/tokens".into()),
        }],
        badge: Some("rate limited".into()),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// search: instant tier. kv cache only - never touches the network.
// ---------------------------------------------------------------------------

#[plugin_fn]
pub fn search(input: Json<SearchInput>) -> FnResult<Json<SearchOutput>> {
    // The host strips the trigger ("gh tokio" -> "tokio") before we see it.
    let term = input.0.query.trim().to_lowercase();
    if term.is_empty() {
        return Ok(Json(SearchOutput::default()));
    }
    // Qualifier-only queries ("language:rust") still stream via `query`;
    // match the cache on the free-text part.
    let needle = term
        .split_whitespace()
        .filter(|w| !w.contains(':'))
        .collect::<Vec<_>>()
        .join(" ");

    let cache = read_cache();
    if cache.is_empty() {
        return Ok(Json(SearchOutput {
            results: vec![ExtensionResult {
                id: format!("gh:searching:{term}"),
                title: format!("Search GitHub for \u{201c}{}\u{201d}", input.0.query.trim()),
                subtitle: Some("cache warming - live results stream in".into()),
                relevance: 10.0,
                badge: Some("live".into()),
                ..Default::default()
            }],
        }));
    }

    let mut results: Vec<ExtensionResult> = cache
        .iter()
        .filter_map(|r| {
            if needle.is_empty() {
                return None;
            }
            let name = r.full_name.to_lowercase();
            // Prefix of name > substring of name > description hit.
            let relevance = if name.starts_with(&needle)
                || name.split('/').nth(1).is_some_and(|n| n.starts_with(&needle))
            {
                90.0
            } else if name.contains(&needle) {
                70.0
            } else if r
                .description
                .as_deref()
                .is_some_and(|d| d.to_lowercase().contains(&needle))
            {
                40.0
            } else {
                return None;
            };
            Some(repo_result(r, relevance, "cached"))
        })
        .collect();

    results.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(20);
    Ok(Json(SearchOutput { results }))
}

// ---------------------------------------------------------------------------
// query: async streaming tier. Each endpoint's batch is emitted as soon as it
// returns; repo rows reuse the cached ids so they swap in place.
// ---------------------------------------------------------------------------

#[plugin_fn]
pub fn query(input: Json<QueryInput>) -> FnResult<Json<QueryOutput>> {
    let term = input.0.query.trim().to_string();
    if term.is_empty() {
        return Ok(Json(QueryOutput::default()));
    }
    let max = guest::setting_num("max_per_endpoint")
        .ok()
        .flatten()
        .map(|n| n as u32)
        .unwrap_or(6)
        .clamp(1, 15);
    let q = urlencode(&term);

    // Endpoint 1: repositories. GitHub qualifiers pass through verbatim
    // ("tokio language:rust" works).
    match api_get(&format!("/search/repositories?q={q}&per_page={max}"), "application/vnd.github+json") {
        Ok((200, body)) => {
            if let Ok(resp) = serde_json::from_str::<SearchResp<RepoItem>>(&body) {
                let cached: Vec<CachedRepo> = resp.items.iter().map(CachedRepo::from).collect();
                let n = cached.len();
                let results: Vec<ExtensionResult> = cached
                    .iter()
                    .enumerate()
                    // Preserve GitHub's best-match order within the batch.
                    .map(|(i, r)| repo_result(r, 95.0 - (i as f32 / n.max(1) as f32) * 30.0, "live"))
                    .collect();
                // Stream the batch; Ok(false) = user typed a new query, stop.
                // A host Err also stops - no point trapping over a lost batch.
                if !guest::emit(results).unwrap_or(false) {
                    return Ok(Json(QueryOutput::default()));
                }
                merge_cache(&cached);
            }
        }
        Ok((status, _)) if is_rate_limited(status) => {
            let _ = guest::debug(&format!("repo search rate limited ({status})"));
            let _ = guest::emit(vec![rate_limit_result()]);
            return Ok(Json(QueryOutput::default()));
        }
        Ok((status, _)) => {
            let _ = guest::debug(&format!("repo search failed ({status})"));
        }
        Err(e) => {
            let _ = guest::debug(&format!("repo search error: {e}"));
        }
    }

    // Endpoint 2: issues & PRs. Independent of endpoint 1's outcome.
    // GitHub's advanced issue search (now the default for authenticated
    // requests) rejects a query unless it names a type, so issues and PRs are
    // fetched as two separate `is:` searches and merged.
    let issues_on = guest::setting_bool("search_issues").ok().flatten().unwrap_or(true);
    if issues_on {
        let mut items: Vec<IssueItem> = Vec::new();
        let mut rate_limited = false;
        for kind in ["is:issue", "is:pull-request"] {
            let (mut batch, limited) = fetch_issues(&term, kind, max);
            items.append(&mut batch);
            rate_limited |= limited;
        }
        if items.is_empty() && rate_limited {
            let _ = guest::emit(vec![rate_limit_result()]);
        } else {
            items.truncate(max as usize);
            let n = items.len();
            let results: Vec<ExtensionResult> = items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    issue_result(item, 60.0 - (i as f32 / n.max(1) as f32) * 30.0)
                })
                .collect();
            let _ = guest::emit(results);
        }
    }

    Ok(Json(QueryOutput::default()))
}

/// Fetch one title-scoped issue-or-PR batch. `kind` is the required advanced-
/// search type qualifier (`is:issue` / `is:pull-request`). Returns the parsed
/// items plus a flag set when the call was rate limited.
fn fetch_issues(term: &str, kind: &str, max: u32) -> (Vec<IssueItem>, bool) {
    let iq = urlencode(&format!("{term} in:title {kind}"));
    match api_get(&format!("/search/issues?q={iq}&per_page={max}"), "application/vnd.github+json") {
        Ok((200, body)) => (
            serde_json::from_str::<SearchResp<IssueItem>>(&body)
                .map(|r| r.items)
                .unwrap_or_default(),
            false,
        ),
        Ok((status, _)) if is_rate_limited(status) => {
            let _ = guest::debug(&format!("issue search rate limited ({status})"));
            (Vec::new(), true)
        }
        Ok((status, body)) => {
            let _ = guest::debug(&format!("issue search failed ({status}): {body}"));
            (Vec::new(), false)
        }
        Err(e) => {
            let _ = guest::debug(&format!("issue search error: {e}"));
            (Vec::new(), false)
        }
    }
}

// ---------------------------------------------------------------------------
// activate: declarative effects only - urls derive from the result id, so no
// state needs stashing and no clipboard/open_url permissions are needed.
// ---------------------------------------------------------------------------

#[plugin_fn]
pub fn activate(input: Json<ActivateInput>) -> FnResult<Json<ActivateOutput>> {
    let id = input.0.result.id;
    let action = input.0.action.as_deref().unwrap_or("open");

    let effects = if let Some(full) = id.strip_prefix("repo:") {
        match action {
            "copy-clone" => vec![
                ActivateEffect::CopyText { text: format!("https://github.com/{full}.git") },
                ActivateEffect::ShowToast { message: format!("Copied clone URL for {full}") },
            ],
            "copy-gh" => vec![
                ActivateEffect::CopyText { text: format!("gh repo clone {full}") },
                ActivateEffect::ShowToast { message: format!("Copied gh clone command for {full}") },
            ],
            _ => vec![ActivateEffect::OpenUrl { url: format!("https://github.com/{full}") }],
        }
    } else if let Some((repo, number)) = id.strip_prefix("issue:").and_then(|s| s.split_once('#')) {
        // GitHub redirects /issues/N to /pull/N when the issue is a PR.
        let url = format!("https://github.com/{repo}/issues/{number}");
        match action {
            "copy-url" => vec![
                ActivateEffect::CopyText { text: url },
                ActivateEffect::ShowToast { message: "Copied URL".into() },
            ],
            _ => vec![ActivateEffect::OpenUrl { url }],
        }
    } else if id == "gh:ratelimit" {
        vec![ActivateEffect::OpenUrl { url: "https://github.com/settings/tokens".into() }]
    } else {
        // "gh:searching:<term>" hint row -> open the web search.
        let term = id.strip_prefix("gh:searching:").unwrap_or("");
        vec![ActivateEffect::OpenUrl {
            url: format!("https://github.com/search?q={}", urlencode(term)),
        }]
    };

    Ok(Json(ActivateOutput { effects }))
}

// ---------------------------------------------------------------------------
// README link rewriting. The raw README uses paths relative to the repo, which
// the host's markdown preview can't resolve - it has no repo base. Rewrite them
// to absolute github.com URLs so images load and links open the right page:
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

/// Join a repo-relative path onto a base URL that has no trailing slash.
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

/// Rewrite an HTML attribute (`src=` / `href=`) whose value is a quoted,
/// repo-relative URL.
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

/// Rewrite markdown `](url)` targets, choosing the image or link base.
fn rewrite_md_links(input: &str, raw_base: &str, blob_base: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b']' && bytes.get(i + 1) == Some(&b'(') {
            if let Some(rel_end) = input[i + 2..].find(')') {
                let inner = &input[i + 2..i + 2 + rel_end];
                // `url "title"` - only the url part is rewritten.
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
        // Not a link start - copy one whole char (UTF-8 safe; `]`/`(` are ASCII
        // so a multibyte lead byte never trips the check above).
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

// ---------------------------------------------------------------------------
// preview: stream Metadata immediately, then the README / issue body as
// Markdown once fetched.
// ---------------------------------------------------------------------------

#[plugin_fn]
pub fn preview(input: Json<PreviewInput>) -> FnResult<Json<PreviewContent>> {
    let id = input.0.result.id;

    if let Some(full) = id.strip_prefix("repo:") {
        return preview_repo(full);
    }
    if let Some((repo, number)) = id.strip_prefix("issue:").and_then(|s| s.split_once('#')) {
        return preview_issue(repo, number);
    }
    Ok(Json(PreviewContent::Metadata {
        items: vec![MetadataItem {
            label: "GitHub".into(),
            value: "select a repo or issue to preview it".into(),
        }],
    }))
}

/// Rate-limited (or otherwise failed) preview: degrade to whatever the kv
/// cache knows about the repo plus an actionable hint, never a bare error.
fn degraded_preview(full: Option<&str>, status: u16) -> PreviewContent {
    let mut items = Vec::new();
    if let Some(full) = full {
        if let Some(r) = read_cache().into_iter().find(|r| r.full_name == full) {
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

fn preview_repo(full: &str) -> FnResult<Json<PreviewContent>> {
    // Instant first paint from the kv cache (no network), refined by the
    // live metadata and README below.
    if let Some(r) = read_cache().into_iter().find(|r| r.full_name == full) {
        let cached = PreviewContent::Metadata {
            items: vec![
                MetadataItem { label: "Stars".into(), value: fmt_count(r.stars) },
                MetadataItem {
                    label: "Language".into(),
                    value: r.language.clone().unwrap_or_else(|| "-".into()),
                },
                MetadataItem {
                    label: "About".into(),
                    value: r.description.clone().unwrap_or_default(),
                },
            ],
        };
        if !guest::emit_preview_update(&cached).unwrap_or(false) {
            return Ok(Json(cached));
        }
    }

    let (status, body) = api_get(&format!("/repos/{full}"), "application/vnd.github+json")?;
    if status != 200 {
        return Ok(Json(degraded_preview(Some(full), status)));
    }
    let detail: RepoDetail = serde_json::from_str(&body).map_err(extism_pdk::Error::from)?;

    let mut items = vec![
        MetadataItem { label: "Stars".into(), value: fmt_count(detail.stargazers_count) },
        MetadataItem { label: "Forks".into(), value: fmt_count(detail.forks_count) },
        MetadataItem { label: "Open issues".into(), value: fmt_count(detail.open_issues_count) },
    ];
    if let Some(l) = &detail.language {
        items.push(MetadataItem { label: "Language".into(), value: l.clone() });
    }
    if let Some(l) = &detail.license {
        let name = l.spdx_id.clone().filter(|s| s != "NOASSERTION").unwrap_or_else(|| l.name.clone());
        items.push(MetadataItem { label: "License".into(), value: name });
    }
    items.push(MetadataItem { label: "Default branch".into(), value: detail.default_branch.clone() });
    if let Some(ts) = &detail.pushed_at {
        items.push(MetadataItem { label: "Last push".into(), value: humanize(ts) });
    }
    if let Some(d) = detail.description.as_deref().filter(|d| !d.is_empty()) {
        items.push(MetadataItem { label: "About".into(), value: d.to_string() });
    }
    let metadata = PreviewContent::Metadata { items };

    // Show the stats immediately; false = selection moved on, skip the README.
    if !guest::emit_preview_update(&metadata).unwrap_or(false) {
        return Ok(Json(metadata));
    }

    match api_get(&format!("/repos/{full}/readme"), "application/vnd.github.raw+json") {
        Ok((200, readme)) if !readme.trim().is_empty() => {
            // Rewrite repo-relative paths to absolute URLs before truncating.
            let mut readme = absolutize_readme(&readme, full, &detail.default_branch);
            if readme.len() > README_CAP {
                let mut cut = README_CAP;
                while !readme.is_char_boundary(cut) {
                    cut -= 1;
                }
                readme.truncate(cut);
                readme.push_str("\n\n\u{2026}");
            }
            Ok(Json(PreviewContent::Markdown {
                content: format!("# {}\n\n{readme}", detail.full_name),
            }))
        }
        // No README (404) or fetch error: the metadata stands as the preview.
        _ => Ok(Json(metadata)),
    }
}

fn preview_issue(repo: &str, number: &str) -> FnResult<Json<PreviewContent>> {
    let (status, body) = api_get(&format!("/repos/{repo}/issues/{number}"), "application/vnd.github+json")?;
    if status != 200 {
        return Ok(Json(degraded_preview(None, status)));
    }
    let detail: IssueDetail = serde_json::from_str(&body).map_err(extism_pdk::Error::from)?;

    let mut items = vec![
        MetadataItem { label: "State".into(), value: detail.state.clone() },
        MetadataItem { label: "Comments".into(), value: fmt_count(detail.comments) },
    ];
    if let Some(u) = &detail.user {
        items.push(MetadataItem { label: "Author".into(), value: u.login.clone() });
    }
    if !detail.labels.is_empty() {
        let names: Vec<&str> = detail.labels.iter().map(|l| l.name.as_str()).collect();
        items.push(MetadataItem { label: "Labels".into(), value: names.join(", ") });
    }
    if let Some(ts) = &detail.created_at {
        items.push(MetadataItem { label: "Opened".into(), value: humanize(ts) });
    }
    if let Some(ts) = &detail.updated_at {
        items.push(MetadataItem { label: "Updated".into(), value: humanize(ts) });
    }
    let metadata = PreviewContent::Metadata { items };

    let body_md = detail.body.as_deref().unwrap_or("").trim();
    if body_md.is_empty() || !guest::emit_preview_update(&metadata).unwrap_or(false) {
        return Ok(Json(metadata));
    }
    Ok(Json(PreviewContent::Markdown {
        content: format!("# {} ({})\n\n{}", detail.title, detail.state, body_md),
    }))
}

// ---------------------------------------------------------------------------
// refresh: daily "my repos" cache warm (also runs once at load). Without a
// token this is a no-op - the cache still fills lazily from `query` batches.
// Never errors: a failed warm must not disable the extension.
// ---------------------------------------------------------------------------

#[plugin_fn]
pub fn refresh(_input: Json<RefreshInput>) -> FnResult<Json<RefreshOutput>> {
    if token().is_none() {
        return Ok(Json(RefreshOutput::default()));
    }
    match api_get("/user/repos?sort=pushed&per_page=100", "application/vnd.github+json") {
        Ok((200, body)) => {
            if let Ok(repos) = serde_json::from_str::<Vec<RepoItem>>(&body) {
                let cached: Vec<CachedRepo> = repos.iter().map(CachedRepo::from).collect();
                merge_cache(&cached);
                let _ = guest::debug(&format!("warmed cache with {} repos", cached.len()));
            }
        }
        Ok((status, _)) => {
            let _ = guest::debug(&format!("repo warm failed ({status})"));
        }
        Err(e) => {
            let _ = guest::debug(&format!("repo warm error: {e}"));
        }
    }
    Ok(Json(RefreshOutput::default()))
}
