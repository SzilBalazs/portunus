//! Pure presentation: icons, number/time formatting, and the builders that turn
//! wire/cached rows into `ExtensionResult`s with their action menus. No network,
//! no kv - callers hand in already-fetched data.

use crate::api::*;
use portunus_ext_sdk::{Action, ExtensionResult, ResultIcon};

// Octicon glyphs (base64 PNG) shown next to each row, colored to GitHub's
// semantics: repo blue, PR purple, issue green, star gold, bell orange,
// code cyan, user gray.
const ICON_REPO_B64: &str = include_str!("../icon_repo.b64");
const ICON_PR_B64: &str = include_str!("../icon_pr.b64");
const ICON_ISSUE_B64: &str = include_str!("../icon_issue.b64");
const ICON_STAR_B64: &str = include_str!("../icon_star.b64");
const ICON_BELL_B64: &str = include_str!("../icon_bell.b64");
const ICON_CODE_B64: &str = include_str!("../icon_code.b64");
const ICON_USER_B64: &str = include_str!("../icon_user.b64");

fn png_icon(b64: &str) -> ResultIcon {
    ResultIcon { mime: "image/png".into(), data_base64: b64.trim().to_string() }
}

pub fn icon_repo() -> ResultIcon {
    png_icon(ICON_REPO_B64)
}
pub fn icon_star() -> ResultIcon {
    png_icon(ICON_STAR_B64)
}
pub fn icon_bell() -> ResultIcon {
    png_icon(ICON_BELL_B64)
}
pub fn icon_code() -> ResultIcon {
    png_icon(ICON_CODE_B64)
}
pub fn icon_user() -> ResultIcon {
    png_icon(ICON_USER_B64)
}
pub fn icon_issue_or_pr(is_pr: bool) -> ResultIcon {
    png_icon(if is_pr { ICON_PR_B64 } else { ICON_ISSUE_B64 })
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

pub fn fmt_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_949 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

/// Parse an ISO 8601 timestamp ("2024-05-01T12:30:00Z") into unix seconds.
pub fn parse_iso(ts: &str) -> Option<i64> {
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
pub fn humanize(ts: &str) -> String {
    let Some(then) = parse_iso(ts) else { return ts.to_string() };
    let now = portunus_ext_sdk::guest::now().map(|ms| (ms / 1_000) as i64).unwrap_or(then);
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

/// "open" / "merged" / "closed" / "draft" from a PR's state flags.
pub fn pr_state(is_open: bool, merged: bool, draft: bool) -> &'static str {
    if merged {
        "merged"
    } else if !is_open {
        "closed"
    } else if draft {
        "draft"
    } else {
        "open"
    }
}

// ---------------------------------------------------------------------------
// Action menus. First action is the Enter default; the rest reach the action
// picker (Alt+Enter). Ids are matched in `activate`.
// ---------------------------------------------------------------------------

pub fn repo_actions() -> Vec<Action> {
    vec![
        Action { id: "open".into(), label: "Open on GitHub".into(), hint: Some("in browser".into()) },
        Action { id: "open-issues".into(), label: "Open Issues".into(), hint: None },
        Action { id: "open-prs".into(), label: "Open Pull Requests".into(), hint: None },
        Action { id: "open-releases".into(), label: "Open Releases".into(), hint: None },
        Action { id: "open-actions".into(), label: "Open Actions".into(), hint: None },
        Action { id: "copy-clone".into(), label: "Copy clone URL".into(), hint: Some("respects protocol setting".into()) },
        Action { id: "copy-ssh".into(), label: "Copy SSH clone URL".into(), hint: None },
        Action { id: "copy-gh".into(), label: "Copy gh clone command".into(), hint: Some("gh repo clone".into()) },
        Action { id: "copy-url".into(), label: "Copy repo URL".into(), hint: None },
    ]
}

fn issue_actions(is_pr: bool) -> Vec<Action> {
    let kind = if is_pr { "PR" } else { "issue" };
    vec![
        Action { id: "open".into(), label: "Open on GitHub".into(), hint: Some("in browser".into()) },
        Action { id: "copy-url".into(), label: "Copy URL".into(), hint: None },
        Action { id: "copy-number".into(), label: format!("Copy {kind} number"), hint: None },
        Action { id: "copy-title".into(), label: "Copy title".into(), hint: None },
        Action { id: "copy-md".into(), label: "Copy Markdown link".into(), hint: Some("[title](url)".into()) },
    ]
}

fn user_actions() -> Vec<Action> {
    vec![
        Action { id: "open".into(), label: "Open profile".into(), hint: Some("in browser".into()) },
        Action { id: "copy-url".into(), label: "Copy profile URL".into(), hint: None },
    ]
}

fn notif_actions() -> Vec<Action> {
    vec![
        Action { id: "open".into(), label: "Open on GitHub".into(), hint: Some("in browser".into()) },
        Action { id: "open-inbox".into(), label: "Open notification inbox".into(), hint: None },
        Action { id: "copy-url".into(), label: "Copy URL".into(), hint: None },
    ]
}

// ---------------------------------------------------------------------------
// Result builders. Ids encode everything `activate`/`preview` need so no search
// state is persisted (the host hands the whole result back verbatim).
//   repo:owner/name         issue:owner/name#N        pr:owner/name#N
//   user:login              notif|url|type|reason|repo|ts
// ---------------------------------------------------------------------------

pub fn repo_result(r: &CachedRepo, relevance: f32, badge: &str) -> ExtensionResult {
    ExtensionResult {
        id: format!("repo:{}", r.full_name),
        title: r.full_name.clone(),
        subtitle: Some(repo_subtitle(r.stars, r.language.as_deref(), r.description.as_deref())),
        relevance,
        actions: repo_actions(),
        icon: Some(icon_repo()),
        badge: Some(if r.private { "private".into() } else { badge.into() }),
    }
}

pub fn starred_result(r: &CachedRepo, relevance: f32) -> ExtensionResult {
    ExtensionResult {
        id: format!("repo:{}", r.full_name),
        title: r.full_name.clone(),
        subtitle: Some(repo_subtitle(r.stars, r.language.as_deref(), r.description.as_deref())),
        relevance,
        actions: repo_actions(),
        icon: Some(icon_star()),
        badge: Some("starred".into()),
    }
}

pub fn issue_result(i: &CachedIssue, relevance: f32) -> ExtensionResult {
    let kind = if i.is_pr { "PR" } else { "issue" };
    let prefix = if i.is_pr { "pr" } else { "issue" };
    let state = if i.is_pr {
        pr_state(i.state == "open", i.merged, i.draft)
    } else {
        i.state.as_str()
    };
    let badge = if i.badge.is_empty() { state.to_string() } else { i.badge.clone() };
    ExtensionResult {
        id: format!("{prefix}:{}#{}", i.repo, i.number),
        title: i.title.clone(),
        subtitle: Some(format!(
            "{state} {kind} \u{b7} {}#{} \u{b7} {} comments",
            i.repo,
            i.number,
            fmt_count(i.comments)
        )),
        relevance,
        actions: issue_actions(i.is_pr),
        icon: Some(icon_issue_or_pr(i.is_pr)),
        badge: Some(badge),
    }
}

/// Project a live issue-search row into the cached shape, then into a result.
pub fn issue_item_to_cached(i: &IssueItem, badge: &str) -> Option<CachedIssue> {
    let repo = repo_from_api_url(&i.repository_url)?;
    let is_pr = i.pull_request.is_some();
    let merged = i.pull_request.as_ref().and_then(|p| p.merged_at.as_ref()).is_some();
    Some(CachedIssue {
        repo,
        number: i.number,
        title: i.title.clone(),
        state: i.state.clone(),
        is_pr,
        merged,
        draft: i.draft,
        comments: i.comments,
        badge: badge.to_string(),
    })
}

pub fn user_result(u: &UserItem, relevance: f32) -> ExtensionResult {
    let kind = if u.kind.eq_ignore_ascii_case("organization") { "organization" } else { "user" };
    ExtensionResult {
        id: format!("user:{}", u.login),
        title: u.login.clone(),
        subtitle: Some(format!("GitHub {kind}")),
        relevance,
        actions: user_actions(),
        icon: Some(icon_user()),
        badge: Some(kind.into()),
    }
}

pub fn code_result(c: &CodeItem, relevance: f32) -> ExtensionResult {
    ExtensionResult {
        // The html_url already points at the exact file+line; carry it in the id.
        id: format!("code:{}", c.html_url),
        title: c.name.clone(),
        subtitle: Some(format!("{} \u{b7} {}", c.repository.full_name, c.path)),
        relevance,
        actions: vec![
            Action { id: "open".into(), label: "Open file".into(), hint: Some("in browser".into()) },
            Action { id: "copy-url".into(), label: "Copy URL".into(), hint: None },
        ],
        icon: Some(icon_code()),
        badge: Some("code".into()),
    }
}

pub fn notif_result(n: &CachedNotif, relevance: f32) -> ExtensionResult {
    let when = n.updated_at.as_deref().map(humanize).unwrap_or_default();
    let mut sub = format!("{} \u{b7} {} \u{b7} {}", n.kind, n.reason.replace('_', " "), n.repo);
    if !when.is_empty() {
        sub.push_str(&format!(" \u{b7} {when}"));
    }
    ExtensionResult {
        id: format!("notif|{}|{}|{}|{}", n.html_url, n.kind, n.reason, n.repo),
        title: n.title.clone(),
        subtitle: Some(sub),
        relevance,
        actions: notif_actions(),
        icon: Some(icon_bell()),
        badge: Some(n.reason.replace('_', " ")),
    }
}

// ---------------------------------------------------------------------------
// Status / guard rows
// ---------------------------------------------------------------------------

/// Shown when a token-gated command has no token: actionable, opens the PAT page.
pub fn notoken_result(cmd: &str) -> ExtensionResult {
    ExtensionResult {
        id: "gh:notoken".into(),
        title: "GitHub token required".into(),
        subtitle: Some(format!("\u{201c}{cmd}\u{201d} needs a personal access token - add one in extension settings")),
        relevance: 100.0,
        actions: vec![Action {
            id: "open".into(),
            label: "Open token settings".into(),
            hint: Some("github.com/settings/tokens".into()),
        }],
        icon: None,
        badge: Some("setup".into()),
    }
}

pub fn rate_limit_result() -> ExtensionResult {
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
        icon: None,
        badge: Some("rate limited".into()),
    }
}

/// Placeholder shown while the live `query` tier is still fetching. `mode` picks
/// the id namespace so Enter opens the matching web search before results land.
pub fn searching_row(term: &str, mode: &str) -> ExtensionResult {
    ExtensionResult {
        id: format!("gh:searching:{mode}:{term}"),
        title: format!("Search GitHub for \u{201c}{term}\u{201d}"),
        subtitle: Some("live results stream in".into()),
        relevance: 10.0,
        badge: Some("live".into()),
        ..Default::default()
    }
}

/// Empty-dashboard placeholder ("no open pull requests"), shown when a warmed
/// "my" cache is empty so the scope is never blank.
pub fn empty_row(label: &str) -> ExtensionResult {
    ExtensionResult {
        id: "gh:empty".into(),
        title: label.to_string(),
        subtitle: Some("nothing to show".into()),
        relevance: 5.0,
        ..Default::default()
    }
}
