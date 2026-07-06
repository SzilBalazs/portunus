//! Portunus GitHub extension. A read-first GitHub launcher: five live search
//! scopes (repos, issues, PRs, code, users) and five token-gated "my" dashboards
//! (my PRs, my issues, my repos, starred, notifications), each its own launcher
//! command. Sandbox `activate` only opens urls / copies text / toasts, so this
//! never mutates GitHub - "act" always means "open the right page".
//!
//! Tiers, per command:
//!   search   instant, offline - filters a kv cache warmed by `refresh`/`query`
//!   query    async, streaming - hits api.github.com, emits batches, warms cache
//!   preview  streams Metadata then Markdown (README / issue / PR body)
//!   refresh  daily - warms my-repos + starred so dashboards paint on entry
//!
//! The wire `command` field routes every export; result ids encode everything
//! `activate`/`preview` need so no search state is persisted.

mod api;
mod preview;
mod render;

use api::*;
// Bring `extism_pdk` into crate-root scope: the `#[plugin_fn]` macro expands to
// code that references it unqualified.
use portunus_ext_sdk::guest::extism_pdk;
use portunus_ext_sdk::guest::{self, plugin_fn, FnResult, Json};
use portunus_ext_sdk::{
    ActivateEffect, ActivateInput, ActivateOutput, ExtensionResult, PreviewInput, QueryInput,
    QueryOutput, RefreshInput, RefreshOutput, SearchInput, SearchOutput,
};
use render::*;

/// Commands that require a personal access token (private/user-scoped data).
const TOKEN_GATED: [&str; 6] = ["code", "my-prs", "my-issues", "my-repos", "starred", "notifications"];

/// Max rows a dashboard ("my"/starred/notifications) shows at once.
const DASHBOARD_CAP: usize = 40;

fn max_per_endpoint() -> u32 {
    guest::setting_num("max_per_endpoint").ok().flatten().map(|n| n as u32).unwrap_or(6).clamp(1, 15)
}

/// Descending relevance across a batch, preserving GitHub's returned order.
fn ranked(i: usize, n: usize, top: f32, span: f32) -> f32 {
    top - (i as f32 / n.max(1) as f32) * span
}

fn matches(term: &str, haystacks: &[&str]) -> bool {
    if term.is_empty() {
        return true;
    }
    let needle = term.to_lowercase();
    haystacks.iter().any(|h| h.to_lowercase().contains(&needle))
}

// ===========================================================================
// search: instant tier. kv cache only, never the network. Dispatches on the
// wire `command`.
// ===========================================================================

#[plugin_fn]
pub fn search(input: Json<SearchInput>) -> FnResult<Json<SearchOutput>> {
    let cmd = input.0.command.as_str();
    let term = input.0.query.trim().to_string();

    // Token-gated command with no token: one actionable row, no network attempt.
    if TOKEN_GATED.contains(&cmd) && token().is_none() {
        return Ok(Json(SearchOutput { results: vec![notoken_result(&command_title(cmd))] }));
    }

    let results = match cmd {
        "issues" => vec![searching_row(&term, "issue")],
        "prs" => vec![searching_row(&term, "pr")],
        "code" => vec![searching_row(&term, "code")],
        "users" => vec![searching_row(&term, "user")],
        "my-prs" => cached_issues(CACHE_MY_PRS, &term),
        "my-issues" => cached_issues(CACHE_MY_ISSUES, &term),
        "starred" => cached_repos(CACHE_STARRED, &term, true),
        "my-repos" => cached_repos(CACHE_REPOS, &term, false),
        "notifications" => cached_notifs(&term),
        // "repos" and any unknown command default to the repo cache.
        _ => search_repos_cache(&term),
    };
    Ok(Json(SearchOutput { results }))
}

fn command_title(cmd: &str) -> String {
    match cmd {
        "code" => "Search Code",
        "my-prs" => "My Pull Requests",
        "my-issues" => "My Issues",
        "my-repos" => "My Repositories",
        "starred" => "Starred Repositories",
        "notifications" => "Notifications",
        other => other,
    }
    .to_string()
}

/// Instant repo results from the shared repo cache (the `repos` scope).
fn search_repos_cache(term: &str) -> Vec<ExtensionResult> {
    let needle = term
        .to_lowercase()
        .split_whitespace()
        .filter(|w| !w.contains(':'))
        .collect::<Vec<_>>()
        .join(" ");
    let cache = cache_read::<CachedRepo>(CACHE_REPOS);
    if cache.is_empty() || needle.is_empty() {
        return vec![searching_row(term, "repo")];
    }
    let mut results: Vec<ExtensionResult> = cache
        .iter()
        .filter_map(|r| {
            let name = r.full_name.to_lowercase();
            let relevance = if name.starts_with(&needle)
                || name.split('/').nth(1).is_some_and(|n| n.starts_with(&needle))
            {
                90.0
            } else if name.contains(&needle) {
                70.0
            } else if r.description.as_deref().is_some_and(|d| d.to_lowercase().contains(&needle)) {
                40.0
            } else {
                return None;
            };
            Some(repo_result(r, relevance, "cached"))
        })
        .collect();
    // No cache hit yet: show the live-search affordance until `query` streams in.
    if results.is_empty() {
        return vec![searching_row(term, "repo")];
    }
    results.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(20);
    results
}

/// Dashboard repos (my-repos / starred): filter the cache by free text; empty
/// term lists everything. Empty cache returns nothing - the query tier fills it.
fn cached_repos(key: &str, term: &str, starred: bool) -> Vec<ExtensionResult> {
    let cache = cache_read::<CachedRepo>(key);
    let mut out: Vec<ExtensionResult> = cache
        .iter()
        .filter(|r| matches(term, &[&r.full_name, r.description.as_deref().unwrap_or("")]))
        .take(DASHBOARD_CAP)
        .enumerate()
        .map(|(i, r)| {
            let rel = ranked(i, cache.len(), 90.0, 40.0);
            if starred { starred_result(r, rel) } else { repo_result(r, rel, "mine") }
        })
        .collect();
    out.truncate(DASHBOARD_CAP);
    out
}

fn cached_issues(key: &str, term: &str) -> Vec<ExtensionResult> {
    let cache = cache_read::<CachedIssue>(key);
    cache
        .iter()
        .filter(|i| matches(term, &[&i.title, &i.repo]))
        .take(DASHBOARD_CAP)
        .enumerate()
        .map(|(i, row)| issue_result(row, ranked(i, cache.len(), 90.0, 40.0)))
        .collect()
}

fn cached_notifs(term: &str) -> Vec<ExtensionResult> {
    let cache = cache_read::<CachedNotif>(CACHE_NOTIFS);
    cache
        .iter()
        .filter(|n| matches(term, &[&n.title, &n.repo]))
        .take(DASHBOARD_CAP)
        .enumerate()
        .map(|(i, n)| notif_result(n, ranked(i, cache.len(), 90.0, 40.0)))
        .collect()
}

// ===========================================================================
// query: async streaming tier. Hits api.github.com, streams batches, warms the
// caches. Dispatches on the wire `command`.
// ===========================================================================

#[plugin_fn]
pub fn query(input: Json<QueryInput>) -> FnResult<Json<QueryOutput>> {
    let cmd = input.0.command.as_str();
    let term = input.0.query.trim().to_string();

    // Token-gated: `search` already showed the setup row; do nothing here so it
    // isn't cleared by an empty emit.
    if TOKEN_GATED.contains(&cmd) && token().is_none() {
        return Ok(Json(QueryOutput::default()));
    }

    let max = max_per_endpoint();
    match cmd {
        "issues" if !term.is_empty() => query_issue_search(&term, "is:issue", max),
        "prs" if !term.is_empty() => query_issue_search(&term, "is:pr", max),
        "code" if !term.is_empty() => query_code(&term, max),
        "users" if !term.is_empty() => query_users(&term, max),
        "repos" if !term.is_empty() => query_repos(&term, max),
        "my-prs" => query_my_issues(CACHE_MY_PRS, MY_PR_QUERIES, &term),
        "my-issues" => query_my_issues(CACHE_MY_ISSUES, MY_ISSUE_QUERIES, &term),
        "my-repos" => query_my_repos(&term),
        "starred" => query_starred(&term),
        "notifications" => query_notifications(&term),
        _ => {}
    }
    Ok(Json(QueryOutput::default()))
}

fn query_repos(term: &str, max: u32) {
    let q = urlencode(term);
    match api_get(&format!("/search/repositories?q={q}&per_page={max}"), ACCEPT_JSON) {
        Ok((200, body)) => {
            if let Ok(resp) = serde_json::from_str::<SearchResp<RepoItem>>(&body) {
                let cached: Vec<CachedRepo> = resp.items.iter().map(CachedRepo::from).collect();
                let n = cached.len();
                let results: Vec<ExtensionResult> = cached
                    .iter()
                    .enumerate()
                    .map(|(i, r)| repo_result(r, ranked(i, n, 95.0, 30.0), "live"))
                    .collect();
                if !guest::emit(results).unwrap_or(false) {
                    return;
                }
                merge_repo_cache(&cached);
            }
        }
        Ok((s, _)) if is_rate_limited(s) => emit_rate_limited("repo search", s),
        Ok((s, _)) => log_status("repo search", s),
        Err(e) => log_err("repo search", &e),
    }
}

/// The `issues`/`prs` scopes: one title-scoped /search/issues call. The advanced
/// issue search rejects a query without a type, so `is:issue`/`is:pr` is required.
fn query_issue_search(term: &str, kind_qual: &str, max: u32) {
    let q = urlencode(&format!("{term} in:title {kind_qual}"));
    match api_get(&format!("/search/issues?q={q}&per_page={max}"), ACCEPT_JSON) {
        Ok((200, body)) => {
            if let Ok(resp) = serde_json::from_str::<SearchResp<IssueItem>>(&body) {
                let n = resp.items.len();
                let results: Vec<ExtensionResult> = resp
                    .items
                    .iter()
                    .enumerate()
                    .filter_map(|(i, item)| {
                        issue_item_to_cached(item, "").map(|c| issue_result(&c, ranked(i, n, 80.0, 30.0)))
                    })
                    .collect();
                let _ = guest::emit(results);
            }
        }
        Ok((s, _)) if is_rate_limited(s) => emit_rate_limited("issue search", s),
        Ok((s, _)) => log_status("issue search", s),
        Err(e) => log_err("issue search", &e),
    }
}

fn query_code(term: &str, max: u32) {
    let q = urlencode(term);
    match api_get(&format!("/search/code?q={q}&per_page={max}"), ACCEPT_JSON) {
        Ok((200, body)) => {
            if let Ok(resp) = serde_json::from_str::<SearchResp<CodeItem>>(&body) {
                let n = resp.items.len();
                let results: Vec<ExtensionResult> = resp
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, c)| code_result(c, ranked(i, n, 80.0, 30.0)))
                    .collect();
                let _ = guest::emit(results);
            }
        }
        Ok((s, _)) if is_rate_limited(s) => emit_rate_limited("code search", s),
        Ok((s, _)) => log_status("code search", s),
        Err(e) => log_err("code search", &e),
    }
}

fn query_users(term: &str, max: u32) {
    let q = urlencode(term);
    match api_get(&format!("/search/users?q={q}&per_page={max}"), ACCEPT_JSON) {
        Ok((200, body)) => {
            if let Ok(resp) = serde_json::from_str::<SearchResp<UserItem>>(&body) {
                let n = resp.items.len();
                let results: Vec<ExtensionResult> = resp
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, u)| user_result(u, ranked(i, n, 80.0, 30.0)))
                    .collect();
                let _ = guest::emit(results);
            }
        }
        Ok((s, _)) if is_rate_limited(s) => emit_rate_limited("user search", s),
        Ok((s, _)) => log_status("user search", s),
        Err(e) => log_err("user search", &e),
    }
}

// Canned dashboard queries: (badge, qualifier). `@me` resolves to the token's
// user server-side. Newest first via sort=updated.
const MY_PR_QUERIES: &[(&str, &str)] = &[
    ("mine", "is:pr is:open author:@me"),
    ("review", "is:pr is:open review-requested:@me"),
    ("assigned", "is:pr is:open assignee:@me"),
];
const MY_ISSUE_QUERIES: &[(&str, &str)] = &[
    ("mine", "is:issue is:open author:@me"),
    ("assigned", "is:issue is:open assignee:@me"),
    ("mentioned", "is:issue is:open mentions:@me"),
];

fn query_my_issues(cache_key: &str, queries: &[(&str, &str)], term: &str) {
    let mut rows: Vec<CachedIssue> = Vec::new();
    let mut rate_limited = false;
    for (badge, qual) in queries {
        let q = urlencode(qual);
        match api_get(&format!("/search/issues?q={q}&sort=updated&order=desc&per_page=25"), ACCEPT_JSON) {
            Ok((200, body)) => {
                if let Ok(resp) = serde_json::from_str::<SearchResp<IssueItem>>(&body) {
                    for item in &resp.items {
                        if let Some(c) = issue_item_to_cached(item, badge) {
                            // Dedupe across the canned queries; first badge wins.
                            if !rows.iter().any(|r| r.repo == c.repo && r.number == c.number) {
                                rows.push(c);
                            }
                        }
                    }
                }
            }
            Ok((s, _)) if is_rate_limited(s) => rate_limited = true,
            Ok((s, _)) => log_status("my issues", s),
            Err(e) => log_err("my issues", &e),
        }
    }
    rows.truncate(DASHBOARD_CAP);
    cache_write(cache_key, &rows);
    emit_dashboard_issues(&rows, term, rate_limited);
}

fn query_my_repos(term: &str) {
    match api_get("/user/repos?sort=pushed&per_page=100", ACCEPT_JSON) {
        Ok((200, body)) => {
            if let Ok(repos) = serde_json::from_str::<Vec<RepoItem>>(&body) {
                let cached: Vec<CachedRepo> = repos.iter().map(CachedRepo::from).collect();
                merge_repo_cache(&cached);
                emit_dashboard_repos(&cached, term, false);
            }
        }
        Ok((s, _)) if is_rate_limited(s) => emit_rate_limited("my repos", s),
        Ok((s, _)) => log_status("my repos", s),
        Err(e) => log_err("my repos", &e),
    }
}

fn query_starred(term: &str) {
    match api_get("/user/starred?sort=updated&per_page=100", ACCEPT_JSON) {
        Ok((200, body)) => {
            if let Ok(repos) = serde_json::from_str::<Vec<RepoItem>>(&body) {
                let cached: Vec<CachedRepo> = repos.iter().map(CachedRepo::from).collect();
                cache_write(CACHE_STARRED, &cached);
                emit_dashboard_repos(&cached, term, true);
            }
        }
        Ok((s, _)) if is_rate_limited(s) => emit_rate_limited("starred", s),
        Ok((s, _)) => log_status("starred", s),
        Err(e) => log_err("starred", &e),
    }
}

fn query_notifications(term: &str) {
    match api_get("/notifications?per_page=50", ACCEPT_JSON) {
        Ok((200, body)) => {
            if let Ok(items) = serde_json::from_str::<Vec<NotificationItem>>(&body) {
                let rows: Vec<CachedNotif> = items
                    .iter()
                    .map(|n| CachedNotif {
                        repo: n.repository.full_name.clone(),
                        title: n.subject.title.clone(),
                        kind: n.subject.kind.clone(),
                        reason: n.reason.clone(),
                        html_url: notif_html_url(&n.subject, &n.repository.full_name),
                        updated_at: n.updated_at.clone(),
                    })
                    .collect();
                cache_write(CACHE_NOTIFS, &rows);
                let filtered: Vec<&CachedNotif> =
                    rows.iter().filter(|n| matches(term, &[&n.title, &n.repo])).take(DASHBOARD_CAP).collect();
                if filtered.is_empty() {
                    let _ = guest::emit(vec![empty_row("You're all caught up")]);
                } else {
                    let n = filtered.len();
                    let out: Vec<ExtensionResult> =
                        filtered.iter().enumerate().map(|(i, row)| notif_result(row, ranked(i, n, 90.0, 40.0))).collect();
                    let _ = guest::emit(out);
                }
            }
        }
        Ok((s, _)) if is_rate_limited(s) => emit_rate_limited("notifications", s),
        Ok((s, _)) => log_status("notifications", s),
        Err(e) => log_err("notifications", &e),
    }
}

fn emit_dashboard_repos(rows: &[CachedRepo], term: &str, starred: bool) {
    let filtered: Vec<&CachedRepo> = rows
        .iter()
        .filter(|r| matches(term, &[&r.full_name, r.description.as_deref().unwrap_or("")]))
        .take(DASHBOARD_CAP)
        .collect();
    if filtered.is_empty() {
        let _ = guest::emit(vec![empty_row(if starred { "No starred repositories" } else { "No repositories" })]);
        return;
    }
    let n = filtered.len();
    let out: Vec<ExtensionResult> = filtered
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let rel = ranked(i, n, 90.0, 40.0);
            if starred { starred_result(r, rel) } else { repo_result(r, rel, "mine") }
        })
        .collect();
    let _ = guest::emit(out);
}

fn emit_dashboard_issues(rows: &[CachedIssue], term: &str, rate_limited: bool) {
    let filtered: Vec<&CachedIssue> = rows.iter().filter(|i| matches(term, &[&i.title, &i.repo])).take(DASHBOARD_CAP).collect();
    if filtered.is_empty() {
        let msg = if rate_limited { rate_limit_result() } else { empty_row("Nothing open right now") };
        let _ = guest::emit(vec![msg]);
        return;
    }
    let n = filtered.len();
    let out: Vec<ExtensionResult> = filtered.iter().enumerate().map(|(i, row)| issue_result(row, ranked(i, n, 90.0, 40.0))).collect();
    let _ = guest::emit(out);
}

fn emit_rate_limited(ctx: &str, status: u16) {
    let _ = guest::debug(&format!("{ctx} rate limited ({status})"));
    let _ = guest::emit(vec![rate_limit_result()]);
}
fn log_status(ctx: &str, status: u16) {
    let _ = guest::debug(&format!("{ctx} failed ({status})"));
}
fn log_err(ctx: &str, e: &portunus_ext_sdk::guest::extism_pdk::Error) {
    let _ = guest::debug(&format!("{ctx} error: {e}"));
}

// ===========================================================================
// activate: declarative effects only. Urls derive from the result id + action.
// ===========================================================================

#[plugin_fn]
pub fn activate(input: Json<ActivateInput>) -> FnResult<Json<ActivateOutput>> {
    let action = input.0.action.as_deref().unwrap_or("open");
    let result = &input.0.result;
    let id = result.id.as_str();
    let effects = activate_effects(id, action, &result.title);
    Ok(Json(ActivateOutput { effects }))
}

fn copy_toast(text: String, msg: &str) -> Vec<ActivateEffect> {
    vec![ActivateEffect::CopyText { text }, ActivateEffect::ShowToast { message: msg.into() }]
}

fn open(url: String) -> Vec<ActivateEffect> {
    vec![ActivateEffect::OpenUrl { url }]
}

fn activate_effects(id: &str, action: &str, title: &str) -> Vec<ActivateEffect> {
    if let Some(full) = id.strip_prefix("repo:") {
        let url = format!("https://github.com/{full}");
        return match action {
            "open-issues" => open(format!("{url}/issues")),
            "open-prs" => open(format!("{url}/pulls")),
            "open-releases" => open(format!("{url}/releases")),
            "open-actions" => open(format!("{url}/actions")),
            "copy-url" => copy_toast(url, "Copied repo URL"),
            "copy-clone" => {
                let ssh = guest::setting_str("clone_protocol").ok().flatten().as_deref() == Some("ssh");
                let text = if ssh { format!("git@github.com:{full}.git") } else { format!("https://github.com/{full}.git") };
                copy_toast(text, "Copied clone URL")
            }
            "copy-ssh" => copy_toast(format!("git@github.com:{full}.git"), "Copied SSH clone URL"),
            "copy-gh" => copy_toast(format!("gh repo clone {full}"), "Copied gh clone command"),
            _ => open(url),
        };
    }

    // issue:owner/name#N  and  pr:owner/name#N
    for (prefix, path) in [("pr:", "pull"), ("issue:", "issues")] {
        if let Some((repo, number)) = id.strip_prefix(prefix).and_then(|s| s.split_once('#')) {
            let url = format!("https://github.com/{repo}/{path}/{number}");
            return match action {
                "copy-url" => copy_toast(url, "Copied URL"),
                "copy-number" => copy_toast(format!("#{number}"), "Copied number"),
                "copy-title" => copy_toast(title.to_string(), "Copied title"),
                "copy-md" => copy_toast(format!("[{title}]({url})"), "Copied Markdown link"),
                _ => open(url),
            };
        }
    }

    if let Some(login) = id.strip_prefix("user:") {
        let url = format!("https://github.com/{login}");
        return match action {
            "copy-url" => copy_toast(url, "Copied profile URL"),
            _ => open(url),
        };
    }

    if let Some(html_url) = id.strip_prefix("code:") {
        return match action {
            "copy-url" => copy_toast(html_url.to_string(), "Copied URL"),
            _ => open(html_url.to_string()),
        };
    }

    // notif|url|type|reason|repo
    if let Some(rest) = id.strip_prefix("notif|") {
        let url = rest.split('|').next().unwrap_or("").to_string();
        return match action {
            "open-inbox" => open("https://github.com/notifications".into()),
            "copy-url" => copy_toast(url, "Copied URL"),
            _ => open(url),
        };
    }

    if id == "gh:ratelimit" || id == "gh:notoken" {
        return open("https://github.com/settings/tokens".into());
    }

    if id == "gh:empty" {
        return Vec::new();
    }

    // gh:searching:<mode>:<term> placeholder -> open the matching web search.
    let rest = id.strip_prefix("gh:searching:").unwrap_or("");
    let (mode, term) = rest.split_once(':').unwrap_or(("repo", rest));
    let scope = match mode {
        "issue" => "&type=issues",
        "pr" => "&type=pullrequests",
        "code" => "&type=code",
        "user" => "&type=users",
        _ => "",
    };
    open(format!("https://github.com/search?q={}{scope}", urlencode(term)))
}

// ===========================================================================
// preview: stream Metadata then Markdown.
// ===========================================================================

#[plugin_fn]
pub fn preview(input: Json<PreviewInput>) -> FnResult<Json<portunus_ext_sdk::PreviewContent>> {
    preview::preview(&input.0.result.id)
}

// ===========================================================================
// refresh: warm the my-repos + starred caches so their dashboards paint on
// entry. Token-less = no-op. Never errors: a failed warm must not disable us.
// ===========================================================================

#[plugin_fn]
pub fn refresh(_input: Json<RefreshInput>) -> FnResult<Json<RefreshOutput>> {
    if token().is_none() {
        return Ok(Json(RefreshOutput::default()));
    }
    if let Ok((200, body)) = api_get("/user/repos?sort=pushed&per_page=100", ACCEPT_JSON) {
        if let Ok(repos) = serde_json::from_str::<Vec<RepoItem>>(&body) {
            let cached: Vec<CachedRepo> = repos.iter().map(CachedRepo::from).collect();
            merge_repo_cache(&cached);
            let _ = guest::debug(&format!("warmed {} repos", cached.len()));
        }
    }
    if let Ok((200, body)) = api_get("/user/starred?sort=updated&per_page=100", ACCEPT_JSON) {
        if let Ok(repos) = serde_json::from_str::<Vec<RepoItem>>(&body) {
            let cached: Vec<CachedRepo> = repos.iter().map(CachedRepo::from).collect();
            cache_write(CACHE_STARRED, &cached);
            let _ = guest::debug(&format!("warmed {} starred", cached.len()));
        }
    }
    Ok(Json(RefreshOutput::default()))
}
