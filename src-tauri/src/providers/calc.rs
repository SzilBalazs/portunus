use std::rc::Rc;

use exp_rs::{interp, EvalContext};

use super::{Provider, SearchResult};

pub struct CalcProvider;

impl Provider for CalcProvider {
    fn id(&self) -> &'static str {
        "calc"
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let q = query.trim();
        if q.is_empty() {
            return vec![];
        }
        // Skip queries with no digits or math characters to avoid parsing overhead.
        if !q.chars().any(|c| c.is_ascii_digit() || "+-*/^%().".contains(c)) {
            return vec![];
        }
        let mut ctx = EvalContext::new();
        let _ = ctx.register_native_function("log2", 1, |args| args[0].log2());
        let Ok(result) = interp(q, Some(Rc::new(ctx))) else {
            return vec![];
        };
        if !result.is_finite() {
            return vec![];
        }
        let rounded = (result * 1e10).round() / 1e10;
        let display = if rounded.fract() == 0.0 && rounded.abs() < 1e15 {
            format!("{:.0}", rounded)
        } else {
            let s = format!("{:.10}", rounded);
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        };
        vec![SearchResult {
            id: "calc:result".to_string(),
            title: display,
            subtitle: Some(q.to_string()),
            kind: "calc".to_string(),
            score: super::SCORE_CALC,
            ..Default::default()
        }]
    }
}
