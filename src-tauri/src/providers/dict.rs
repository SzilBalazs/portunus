use std::process::Command;

use serde::Serialize;

use super::{Provider, SearchResult};

#[derive(Debug, Clone, Serialize)]
pub struct Definition {
    pub pos: String,
    pub num: u32,
    pub text: String,
    pub quotes: Vec<String>,
    pub synonyms: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DictResult {
    pub word: String,
    pub definitions: Vec<Definition>,
}

pub struct DictProvider {
    pub available: bool,
}

impl DictProvider {
    pub fn new() -> Self {
        let available = Command::new("dict").arg("--version").output().is_ok();
        Self { available }
    }
}

impl Provider for DictProvider {
    fn id(&self) -> &'static str {
        "dict"
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        if !self.available {
            return vec![];
        }

        let q = query.trim_end();

        // Tier 1: typing a prefix of "define" or "dict" (min 2 chars, no space)
        let is_define_prefix =
            q.len() >= 2 && "define".starts_with(q) && !q.contains(' ');
        let is_dict_prefix =
            q.len() >= 2 && "dict".starts_with(q) && !q.contains(' ');

        if is_define_prefix || is_dict_prefix {
            return vec![hint_result("Look up any word in the WordNet dictionary")];
        }

        // Tier 2: prefix + trailing space but no word
        if q == "define " || q == "dict " {
            return vec![hint_result("Start typing a word to look it up\u{2026}")];
        }

        // Tier 3: actual lookup
        let word = if let Some(w) = q.strip_prefix("define ") {
            w.trim()
        } else if let Some(w) = q.strip_prefix("dict ") {
            w.trim()
        } else {
            return vec![];
        };

        if word.is_empty() {
            return vec![hint_result("Start typing a word to look it up\u{2026}")];
        }

        // Don't call dict here — that would block every keystroke.
        // The preview fetches definitions asynchronously via get_dict_definitions.
        vec![SearchResult {
            id: format!("dict:{word}"),
            title: word.to_string(),
            subtitle: Some("WordNet dictionary".to_string()),
            kind: "dict".to_string(),
            score: super::SCORE_DICT,
            ..Default::default()
        }]
    }
}

fn hint_result(subtitle: &str) -> SearchResult {
    SearchResult {
        id: "dict:hint".to_string(),
        title: "define word".to_string(),
        subtitle: Some(subtitle.to_string()),
        kind: "dict-hint".to_string(),
        score: super::SCORE_DICT,
        ..Default::default()
    }
}

fn is_not_found(output: &str) -> bool {
    output.contains("No definitions found")
        || output.contains("No matches found")
        || output.trim().is_empty()
}

fn expand_pos(code: &str) -> &'static str {
    match code {
        "n" => "Noun",
        "v" => "Verb",
        "adj" | "s" => "Adjective",
        "adv" | "r" => "Adverb",
        _ => "Unknown",
    }
}

fn try_parse_def_start(s: &str) -> Option<(Option<String>, u32, String)> {
    // s is already trimmed of leading whitespace.
    // Patterns: "n 1: text", "2: text", "adv 3: text"
    let mut rest = s;
    let mut pos_code: Option<String> = None;

    // Check for optional POS prefix: lowercase letters then space then digit
    let alpha_len = rest.chars().take_while(|c| c.is_ascii_lowercase()).count();
    if alpha_len > 0 && alpha_len < rest.len() {
        let after_alpha = &rest[alpha_len..];
        if after_alpha.starts_with(' ') {
            let after_space = &after_alpha[1..];
            if after_space.chars().next().map_or(false, |c| c.is_ascii_digit()) {
                pos_code = Some(rest[..alpha_len].to_string());
                rest = after_space;
            }
        }
    }

    // Now rest should start with "N: " where N is one or more digits
    let digit_len = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_len == 0 {
        return None;
    }
    let num: u32 = rest[..digit_len].parse().ok()?;
    let after_num = &rest[digit_len..];
    if !after_num.starts_with(": ") {
        return None;
    }
    Some((pos_code, num, after_num[2..].to_string()))
}

fn extract_synonyms(text: &str) -> Vec<String> {
    let mut syns = Vec::new();
    let mut s = text;
    while let Some(start) = s.find("[syn:") {
        let after = &s[start + 5..];
        let end = match after.find(']') {
            Some(e) => e,
            None => break,
        };
        let content = &after[..end];
        let mut c = content;
        while let Some(brace_start) = c.find('{') {
            c = &c[brace_start + 1..];
            if let Some(brace_end) = c.find('}') {
                let word = c[..brace_end].trim().to_string();
                if !word.is_empty() {
                    syns.push(word);
                }
                c = &c[brace_end + 1..];
            } else {
                break;
            }
        }
        s = &s[start + 5 + end + 1..];
    }
    syns
}

fn extract_quotes(text: &str) -> Vec<String> {
    let mut quotes = Vec::new();
    let mut s = text;
    while let Some(start) = s.find('"') {
        let after = &s[start + 1..];
        if let Some(end) = after.find('"') {
            let q = after[..end].trim().to_string();
            if !q.is_empty() {
                quotes.push(q);
            }
            s = &after[end + 1..];
        } else {
            break;
        }
    }
    quotes
}

fn strip_brackets(text: &str) -> String {
    let mut result = String::new();
    let mut s = text;
    while let Some(start) = s.find('[') {
        result.push_str(&s[..start]);
        if let Some(end) = s[start..].find(']') {
            s = &s[start + end + 1..];
        } else {
            result.push_str(&s[start..]);
            s = "";
            break;
        }
    }
    result.push_str(s);
    result
}

fn strip_inline_quotes(text: &str) -> String {
    let mut result = String::new();
    let mut s = text;
    while let Some(start) = s.find('"') {
        result.push_str(&s[..start]);
        let after = &s[start + 1..];
        if let Some(end) = after.find('"') {
            s = &after[end + 1..];
        } else {
            result.push_str(&s[start..]);
            s = "";
            break;
        }
    }
    result.push_str(s);
    result
}

fn build_def(pos: String, num: u32, raw: &str) -> Definition {
    let synonyms = extract_synonyms(raw);
    let quotes = extract_quotes(raw);

    let no_syn = strip_brackets(raw);
    let no_quotes = strip_inline_quotes(&no_syn);

    // Remove empty semicolon segments and normalize whitespace
    let text = no_quotes
        .split(';')
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("; ");

    Definition { pos, num, text, quotes, synonyms }
}

pub fn parse_full(output: &str, word: &str) -> DictResult {
    let mut defs: Vec<Definition> = Vec::new();
    let mut in_block = false;
    let mut current_pos = "Noun".to_string();
    let mut current_def_pos = current_pos.clone();
    let mut current_num: u32 = 0;
    let mut current_raw: Option<String> = None;

    for line in output.lines() {
        if line.trim_start().starts_with("From WordNet") {
            in_block = true;
            continue;
        }
        // Stop at the next database header (shouldn't happen with -d wn, but be safe)
        if in_block && line.trim_start().starts_with("From ") && !line.trim_start().starts_with("From WordNet") {
            break;
        }
        if !in_block {
            continue;
        }

        let leading = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();

        if trimmed.is_empty() {
            continue;
        }
        // Skip the word header line (indented by only 2 spaces)
        if leading < 4 {
            continue;
        }

        if let Some((pos_opt, num, text)) = try_parse_def_start(trimmed) {
            // Finalize previous definition
            if let Some(raw) = current_raw.take() {
                defs.push(build_def(current_def_pos.clone(), current_num, &raw));
            }
            if let Some(pos) = pos_opt {
                current_pos = expand_pos(&pos).to_string();
            }
            current_def_pos = current_pos.clone();
            current_num = num;
            current_raw = Some(text);
        } else if let Some(ref mut raw) = current_raw {
            raw.push(' ');
            raw.push_str(trimmed);
        }
    }

    // Finalize last definition
    if let Some(raw) = current_raw {
        defs.push(build_def(current_def_pos, current_num, &raw));
    }

    DictResult { word: word.to_string(), definitions: defs }
}

#[tauri::command]
pub async fn get_dict_definitions(word: String) -> Result<DictResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let output = std::process::Command::new("dict")
            .args(["-d", "wn", &word])
            .output()
            .map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if is_not_found(&stdout) {
            return Err("not_found".to_string());
        }
        Ok(parse_full(&stdout, &word))
    })
    .await
    .map_err(|e| e.to_string())?
}
