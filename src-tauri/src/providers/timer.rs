use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{Provider, SearchResult, SCORE_TIMER};

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct TimerEntry {
    pub id: u32,
    pub label: String,
    pub duration_secs: u64,
    pub started_at: u64,
}

pub struct TimerState {
    entries: Mutex<Vec<TimerEntry>>,
    next_id: AtomicU32,
}

impl TimerState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(Vec::new()),
            next_id: AtomicU32::new(1),
        })
    }

    pub fn create(&self, duration_secs: u64, label: String) -> u32 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.entries.lock().unwrap().push(TimerEntry {
            id,
            label,
            duration_secs,
            started_at: unix_now(),
        });
        id
    }

    pub fn stop(&self, id: u32) {
        self.entries.lock().unwrap().retain(|e| e.id != id);
    }

    pub fn drain_expired(&self) -> Vec<TimerEntry> {
        let now = unix_now();
        let mut entries = self.entries.lock().unwrap();
        let mut expired = Vec::new();
        entries.retain(|e| {
            if now >= e.started_at + e.duration_secs {
                expired.push(e.clone());
                false
            } else {
                true
            }
        });
        expired
    }
}

pub struct TimerProvider {
    state: Arc<TimerState>,
}

impl TimerProvider {
    pub fn new(state: Arc<TimerState>) -> Self {
        Self { state }
    }
}

// Parse "30", "30s", "5m", "1h", "1h30m", "1h30m20s" → (total_secs, human_label).
fn parse_duration(s: &str) -> Option<(u64, String)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total: u64 = 0;
    let mut parts: Vec<String> = Vec::new();
    let mut rem = s;

    if let Some(pos) = rem.find('h') {
        if let Ok(h) = rem[..pos].parse::<u64>() {
            if h > 0 {
                total += h * 3600;
                parts.push(format!("{}h", h));
            }
            rem = &rem[pos + 1..];
        }
    }
    if let Some(pos) = rem.find('m') {
        if let Ok(m) = rem[..pos].parse::<u64>() {
            if m > 0 {
                total += m * 60;
                parts.push(format!("{}m", m));
            }
            rem = &rem[pos + 1..];
        }
    }
    let num_str = rem.trim_end_matches('s');
    if !num_str.is_empty() {
        if let Ok(sec) = num_str.parse::<u64>() {
            if sec > 0 {
                total += sec;
                parts.push(format!("{}s", sec));
            }
        }
    }

    if total == 0 {
        return None;
    }
    Some((total, parts.join(" ")))
}

fn fmt_remaining(secs: i64) -> String {
    if secs <= 0 {
        return "0s".to_string();
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}h {}m {}s", h, m, s)
    } else if m > 0 {
        format!("{}m {}s", m, s)
    } else {
        format!("{}s", s)
    }
}

fn hint_result() -> SearchResult {
    SearchResult {
        id: "timer:hint".to_string(),
        title: "Set timer…".to_string(),
        subtitle: Some("e.g.  30s · 5m · 1h30m".to_string()),
        kind: "timer-create".to_string(),
        score: SCORE_TIMER,
        exec: None,
        icon_path: None,
        file_size: None,
        created: None,
        modified: None,
    }
}

impl Provider for TimerProvider {
    fn id(&self) -> &'static str {
        "timer"
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let q = query.trim().to_lowercase();

        // "set timer" or "set timer <duration>"
        if q == "set timer" || q.starts_with("set timer ") {
            let duration_str = q.strip_prefix("set timer ").unwrap_or("").trim();
            if duration_str.is_empty() {
                return vec![hint_result()];
            }
            return match parse_duration(duration_str) {
                Some((secs, label)) => vec![SearchResult {
                    id: "timer:create".to_string(),
                    title: format!("Set timer for {}", label),
                    subtitle: Some("↵ to start".to_string()),
                    kind: "timer-create".to_string(),
                    score: SCORE_TIMER,
                    exec: Some(format!("timer:create:{}:{}", secs, label)),
                    icon_path: None,
                    file_size: Some(secs),
                    created: None,
                    modified: None,
                }],
                None => vec![hint_result()],
            };
        }

        // "ti" .. "timers" prefix — show running timers list
        if q.len() >= 2 && "timers".starts_with(q.as_str()) {
            let entries = self.state.entries.lock().unwrap().clone();
            let now = unix_now();

            let mut results: Vec<SearchResult> = entries
                .iter()
                .map(|e| {
                    let elapsed = now.saturating_sub(e.started_at) as i64;
                    let remaining = (e.duration_secs as i64) - elapsed;
                    SearchResult {
                        id: format!("timer:item:{}", e.id),
                        title: format!("Timer — {}", e.label),
                        subtitle: Some(format!("{} remaining", fmt_remaining(remaining))),
                        kind: "timer-item".to_string(),
                        score: SCORE_TIMER + e.id as f32,
                        exec: Some(format!("timer:stop:{}", e.id)),
                        icon_path: None,
                        file_size: Some(e.duration_secs),
                        created: Some(e.started_at),
                        modified: None,
                    }
                })
                .collect();

            results.push(SearchResult {
                id: "timer:new".to_string(),
                title: "New timer".to_string(),
                subtitle: Some("Type  set timer 5m  to create".to_string()),
                kind: "timer-new".to_string(),
                score: SCORE_TIMER - 500.0,
                exec: Some("timer:new".to_string()),
                icon_path: None,
                file_size: None,
                created: None,
                modified: None,
            });

            return results;
        }

        vec![]
    }
}
