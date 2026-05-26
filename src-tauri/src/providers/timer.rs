use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager};

use super::{PluginRegistry, Provider, SearchResult, SCORE_TIMER};

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

#[derive(serde::Serialize, Clone)]
pub struct TimerExpiredPayload {
    pub id: u32,
    pub label: String,
}

fn start_timer_watcher(app: AppHandle, state: Arc<TimerState>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        for entry in state.drain_expired() {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
            let _ = app.emit(
                "timer-expired",
                TimerExpiredPayload {
                    id: entry.id,
                    label: entry.label,
                },
            );
        }
    });
}

pub fn setup(app: &AppHandle, registry: &Arc<RwLock<PluginRegistry>>) {
    let timer_state = TimerState::new();
    let provider_state = Arc::clone(&timer_state);
    let watcher_state = Arc::clone(&timer_state);
    app.manage(Arc::clone(&timer_state));
    registry.write().unwrap().register(TimerProvider::new(provider_state));
    start_timer_watcher(app.clone(), watcher_state);
}

#[tauri::command]
pub fn create_timer(
    duration_secs: u64,
    label: String,
    timer_state: tauri::State<'_, Arc<TimerState>>,
) -> u32 {
    timer_state.create(duration_secs, label)
}

#[tauri::command]
pub fn stop_timer(id: u32, timer_state: tauri::State<'_, Arc<TimerState>>) {
    timer_state.stop(id);
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
        title: "Start a timer…".to_string(),
        subtitle: Some("30s · 5m · 1h30m".to_string()),
        kind: "timer-hint".to_string(),
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

        // Trigger on "ti".."timer" prefix OR "timer <something>"
        let is_prefix = q.len() >= 2 && "timer".starts_with(q.as_str());
        let is_create = q.starts_with("timer ");

        if !is_prefix && !is_create {
            return vec![];
        }

        let mut results = Vec::new();

        if is_create {
            // Preserve original case for the user's label.
            let rest = query.trim().get(6..).unwrap_or("").trim();
            let mut parts = rest.splitn(2, char::is_whitespace);
            let dur_tok = parts.next().unwrap_or("").trim();
            let label_str = parts.next().unwrap_or("").trim();

            match parse_duration(dur_tok) {
                Some((secs, dur_label)) => {
                    let label = if label_str.is_empty() {
                        dur_label.clone()
                    } else {
                        label_str.to_string()
                    };
                    results.push(SearchResult {
                        id: "timer:create".to_string(),
                        title: format!("Start {} timer", dur_label),
                        subtitle: if label_str.is_empty() {
                            Some("↵ to start".to_string())
                        } else {
                            Some(label_str.to_string())
                        },
                        kind: "timer-create".to_string(),
                        score: SCORE_TIMER + 1.0,
                        exec: Some(format!("timer:create:{}:{}", secs, label)),
                        icon_path: None,
                        file_size: Some(secs),
                        created: None,
                        modified: None,
                    });
                }
                None => results.push(hint_result()),
            }
        } else {
            results.push(hint_result());
        }

        // Append all running timers below the create row.
        let entries = self.state.entries.lock().unwrap().clone();
        let now = unix_now();
        for e in &entries {
            let elapsed = now.saturating_sub(e.started_at) as i64;
            let remaining = (e.duration_secs as i64) - elapsed;
            results.push(SearchResult {
                id: format!("timer:item:{}", e.id),
                title: e.label.clone(),
                subtitle: Some(format!("{} remaining", fmt_remaining(remaining))),
                kind: "timer-item".to_string(),
                score: SCORE_TIMER - e.id as f32,
                exec: Some(format!("timer:stop:{}", e.id)),
                icon_path: None,
                file_size: Some(e.duration_secs),
                created: Some(e.started_at),
                modified: None,
            });
        }

        results
    }
}
