//! GitHub REST plumbing, wire deserialization, and the kv cache that backs the
//! instant `search` tier. Everything that touches api.github.com or the kv
//! store lives here so the command/render/preview layers stay pure data.

use portunus_ext_sdk::guest::extism_pdk::{self, http, HttpRequest};
use portunus_ext_sdk::guest::{self, kv_read, kv_write};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const API: &str = "https://api.github.com";

// kv cache keys. Each holds a JSON array warmed by `refresh`/`query` and served
// instantly (offline) by `search`.
pub const CACHE_REPOS: &str = "repos";
pub const CACHE_STARRED: &str = "starred";
pub const CACHE_MY_PRS: &str = "my_prs";
pub const CACHE_MY_ISSUES: &str = "my_issues";
pub const CACHE_NOTIFS: &str = "notifs";
pub const CACHE_CAP: usize = 200;

/// The user's stored PAT, or `None` when unset/blank. Public search works
/// without one; every "my"/private/code path requires it.
pub fn token() -> Option<String> {
    guest::setting_str("token").ok().flatten().filter(|t| !t.trim().is_empty())
}

/// GET an api.github.com path. Returns (status, body). GitHub rejects requests
/// without a User-Agent; the Authorization header is attached only when a token
/// is stored (unauthenticated = public-only, lower rate limits).
pub fn api_get(path: &str, accept: &str) -> Result<(u16, String), extism_pdk::Error> {
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

/// POST a JSON body to an api.github.com path. Returns (status, body).
/// Requires a stored token (mutations are never anonymous).
pub fn api_post(path: &str, json_body: &str) -> Result<(u16, String), extism_pdk::Error> {
    let mut req = HttpRequest::new(format!("{API}{path}"));
    req.method = Some("POST".into());
    req.headers.insert("User-Agent".into(), "portunus-gh-ext".into());
    req.headers.insert("Accept".into(), ACCEPT_JSON.into());
    req.headers.insert("Content-Type".into(), "application/json".into());
    req.headers.insert("X-GitHub-Api-Version".into(), "2022-11-28".into());
    if let Some(t) = token() {
        req.headers.insert("Authorization".into(), format!("Bearer {t}"));
    }
    let resp = http::request(&req, Some(json_body.as_bytes()))?;
    let body = String::from_utf8_lossy(&resp.body()).into_owned();
    Ok((resp.status_code(), body))
}

/// Default GitHub JSON accept header.
pub const ACCEPT_JSON: &str = "application/vnd.github+json";

pub fn is_rate_limited(status: u16) -> bool {
    status == 403 || status == 429
}

/// Percent-encode a search query for use inside `?q=`.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Wire deserialization: only the fields each endpoint actually needs.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SearchResp<T> {
    #[serde(default = "Vec::new")]
    pub items: Vec<T>,
}

#[derive(Deserialize)]
pub struct RepoItem {
    pub full_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub stargazers_count: u64,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub private: bool,
}

#[derive(Deserialize)]
pub struct IssueItem {
    pub title: String,
    pub number: u64,
    pub state: String,
    #[serde(default)]
    pub comments: u64,
    pub repository_url: String,
    #[serde(default)]
    pub pull_request: Option<PullRequestRef>,
    #[serde(default)]
    pub draft: bool,
}

/// The `pull_request` sub-object present on issue-search rows that are PRs.
/// `merged_at` distinguishes a merged PR from a plain closed one.
#[derive(Deserialize)]
pub struct PullRequestRef {
    #[serde(default)]
    pub merged_at: Option<String>,
}

#[derive(Deserialize)]
pub struct UserItem {
    pub login: String,
    #[serde(default, rename = "type")]
    pub kind: String,
}

#[derive(Deserialize)]
pub struct CodeItem {
    pub name: String,
    pub path: String,
    pub html_url: String,
    pub repository: RepoRef,
}

#[derive(Deserialize)]
pub struct RepoRef {
    pub full_name: String,
}

#[derive(Deserialize)]
pub struct RepoDetail {
    pub full_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub stargazers_count: u64,
    #[serde(default)]
    pub forks_count: u64,
    #[serde(default)]
    pub open_issues_count: u64,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub license: Option<License>,
    #[serde(default)]
    pub default_branch: String,
    #[serde(default)]
    pub pushed_at: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub topics: Vec<String>,
}

#[derive(Deserialize)]
pub struct License {
    #[serde(default)]
    pub spdx_id: Option<String>,
    pub name: String,
}

#[derive(Deserialize)]
pub struct IssueDetail {
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub comments: u64,
    #[serde(default)]
    pub user: Option<Login>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub assignees: Vec<Login>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Deserialize)]
pub struct PrDetail {
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub comments: u64,
    #[serde(default)]
    pub user: Option<Login>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub mergeable_state: Option<String>,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    #[serde(default)]
    pub changed_files: u64,
    #[serde(default)]
    pub commits: u64,
    #[serde(default)]
    pub head: Option<GitRef>,
    #[serde(default)]
    pub base: Option<GitRef>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Deserialize)]
pub struct GitRef {
    #[serde(rename = "ref", default)]
    pub name: String,
}

#[derive(Deserialize)]
pub struct Login {
    pub login: String,
}

#[derive(Deserialize)]
pub struct Label {
    pub name: String,
}

#[derive(Deserialize)]
pub struct UserDetail {
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub followers: u64,
    #[serde(default)]
    pub following: u64,
    #[serde(default)]
    pub public_repos: u64,
    #[serde(default)]
    pub blog: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: String,
}

/// One inbox thread from `/notifications`.
#[derive(Deserialize)]
pub struct NotificationItem {
    pub reason: String,
    pub subject: NotifSubject,
    pub repository: RepoRef,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Deserialize)]
pub struct NotifSubject {
    pub title: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    /// api.github.com resource url; converted to an html url for opening.
    #[serde(default)]
    pub url: Option<String>,
}

// ---------------------------------------------------------------------------
// Cached rows: compact projections stored in kv, rebuilt into results offline.
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
pub struct CachedRepo {
    pub full_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub stars: u64,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub private: bool,
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

/// A cached issue or PR row (the "my issues"/"my prs" dashboards serve these
/// offline). `repo`/`number` reconstruct the id and url; `badge` records which
/// canned query matched (mine / assigned / review / mentioned).
#[derive(Serialize, Deserialize, Clone)]
pub struct CachedIssue {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub is_pr: bool,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub comments: u64,
    #[serde(default)]
    pub badge: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CachedNotif {
    pub repo: String,
    pub title: String,
    pub kind: String,
    pub reason: String,
    pub html_url: String,
    #[serde(default)]
    pub updated_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Generic kv cache
// ---------------------------------------------------------------------------

pub fn cache_read<T: DeserializeOwned>(key: &str) -> Vec<T> {
    kv_read(key)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn cache_write<T: Serialize>(key: &str, rows: &[T]) {
    if let Ok(json) = serde_json::to_string(rows) {
        let _ = kv_write(key, &json);
    }
}

/// Prepend `fresh` repos onto the repo cache, dedupe by full_name, cap size -
/// keeps searched-for repos servable instantly on the next keystroke.
pub fn merge_repo_cache(fresh: &[CachedRepo]) {
    let mut merged: Vec<CachedRepo> = fresh.to_vec();
    for old in cache_read::<CachedRepo>(CACHE_REPOS) {
        if !merged.iter().any(|r| r.full_name == old.full_name) {
            merged.push(old);
        }
    }
    merged.truncate(CACHE_CAP);
    cache_write(CACHE_REPOS, &merged);
}

/// "owner/name" from an api url like ".../repos/owner/name" or
/// ".../repos/owner/name/issues/3" (keeps only the first two path segments).
pub fn repo_from_api_url(url: &str) -> Option<String> {
    let rest = url.split_once("/repos/").map(|(_, r)| r)?;
    let mut it = rest.split('/');
    let owner = it.next()?;
    let name = it.next()?;
    Some(format!("{owner}/{name}"))
}

/// Convert a notification `subject.url` (api.github.com resource) into the
/// browser html url. Handles the common types; unknown shapes fall back to the
/// repo page so Enter always goes somewhere sensible.
pub fn notif_html_url(subject: &NotifSubject, repo: &str) -> String {
    let repo_page = format!("https://github.com/{repo}");
    let Some(url) = subject.url.as_deref() else { return repo_page };
    // .../repos/o/r/pulls/12  -> github.com/o/r/pull/12
    // .../repos/o/r/issues/12 -> github.com/o/r/issues/12
    // .../repos/o/r/releases/123 -> github.com/o/r/releases (id isn't a tag)
    if let Some((_, tail)) = url.split_once("/repos/") {
        let parts: Vec<&str> = tail.split('/').collect();
        if parts.len() >= 4 {
            let base = format!("{}/{}", parts[0], parts[1]);
            let (res, num) = (parts[2], parts[3]);
            return match res {
                "pulls" => format!("https://github.com/{base}/pull/{num}"),
                "issues" => format!("https://github.com/{base}/issues/{num}"),
                "commits" => format!("https://github.com/{base}/commit/{num}"),
                "releases" => format!("https://github.com/{base}/releases"),
                "discussions" => format!("https://github.com/{base}/discussions/{num}"),
                _ => repo_page,
            };
        }
    }
    repo_page
}
