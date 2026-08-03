//! Markdown extraction for the office preview.
//!
//! INTERIM: this module is removed once the HTML renderers land in a later
//! stage. Don't invest in it — it is here unchanged so the split stays a pure
//! move.

use super::pkg::{self, Budget};
use super::xml::{self, attr_local};
use std::path::Path;

// Like extract_office_text, but preserves headings / bold / italic / lists as
// Markdown so the preview can render formatting. Only the formats without an
// HTML renderer reach this: xlsx and pptx go through `office::render`.
pub fn extract_office_markdown(path: &str) -> Result<String, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut budget = Budget::new();
    match ext.as_str() {
        "docx" => extract_docx_markdown(path, &mut budget),
        "odt" | "odp" => extract_odf_markdown(path, &mut budget),
        other => Err(format!("no markdown extractor for {other}")),
    }
}

// Wrap text in Markdown emphasis markers; whitespace-only text is left bare so
// we never emit dangling `** **`.
fn wrap_emphasis(text: &str, bold: bool, italic: bool) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }
    let marker = match (bold, italic) {
        (true, true) => "***",
        (true, false) => "**",
        (false, true) => "*",
        (false, false) => return text.to_string(),
    };
    // Keep surrounding whitespace outside the markers so emphasis stays valid.
    let leading: String = text.chars().take_while(|c| c.is_whitespace()).collect();
    let trailing: String = text
        .chars()
        .rev()
        .take_while(|c| c.is_whitespace())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let core = &text[leading.len()..text.len() - trailing.len()];
    format!("{leading}{marker}{core}{marker}{trailing}")
}

// Escape the Markdown-significant characters that would otherwise turn document
// content into syntax. Minimal on purpose: enough to avoid surprises, not a
// full CommonMark escaper.
fn escape_md(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '\\' | '`' | '*' | '_' | '#' | '[' | ']' | '<' | '>') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

// Escape leading characters that would make a plain paragraph look like a
// markdown block (heading, list, blockquote, ordered list). Input is trimmed.
fn escape_leading_block(s: &str) -> String {
    let b = s.as_bytes();
    // Blockquote.
    if b.first() == Some(&b'>') {
        return format!("\\{s}");
    }
    // Unordered list: `-`, `+`, `*` followed by a space. (`*`/`**` without a
    // space is emphasis from wrap_emphasis, not a list - leave it alone.)
    if matches!(b.first(), Some(b'-') | Some(b'+') | Some(b'*'))
        && matches!(b.get(1), Some(b' ') | None)
    {
        return format!("\\{s}");
    }
    // ATX heading: 1-6 `#` then a space.
    if b.first() == Some(&b'#') {
        let h = b.iter().take_while(|c| **c == b'#').count();
        if h <= 6 && matches!(b.get(h), Some(b' ') | None) {
            return format!("\\{s}");
        }
    }
    // Ordered list: leading digits, then `.` or `)`, then a space.
    let d = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if d > 0
        && matches!(b.get(d), Some(b'.') | Some(b')'))
        && matches!(b.get(d + 1), Some(b' ') | None)
    {
        // Escape the delimiter so `1. x` becomes `1\. x`.
        return format!("{}\\{}", &s[..d], &s[d..]);
    }
    s.to_string()
}

// Map a docx/ODF style name to a heading level (1-6), or None if not a heading.
fn heading_level(style: &str) -> Option<usize> {
    let s = style.to_ascii_lowercase();
    if s == "title" {
        return Some(1);
    }
    let rest = s.strip_prefix("heading").or_else(|| s.strip_prefix("heading "))?;
    let n: usize = rest.trim().parse().ok()?;
    Some(n.clamp(1, 6))
}

fn extract_docx_markdown(path: &str, budget: &mut Budget) -> Result<String, String> {
    let mut zip = pkg::open_zip(path)?;
    // styleId → outline level, used to detect headings independent of the UI
    // language (style names are localized, e.g. "Cmsor1" = Heading 1 in HU).
    let styles_xml = pkg::read_entry(&mut zip, "word/styles.xml", budget)?;
    let xml = pkg::read_entry(&mut zip, "word/document.xml", budget)?
        .ok_or_else(|| "docx: missing word/document.xml".to_string())?;

    let mut outline: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    if let Some(sx) = &styles_xml {
        if let Ok(sdoc) = xml::parse(sx) {
            for st in sdoc
                .root_element()
                .children()
                .filter(|n| n.tag_name().name() == "style")
            {
                let Some(id) = attr_local(st, "styleId") else {
                    continue;
                };
                let lvl = st
                    .children()
                    .find(|n| n.tag_name().name() == "pPr")
                    .and_then(|pr| pr.children().find(|n| n.tag_name().name() == "outlineLvl"))
                    .and_then(|o| attr_local(o, "val"))
                    .and_then(|v| v.parse::<usize>().ok());
                if let Some(l) = lvl {
                    outline.insert(id.to_string(), l);
                }
            }
        }
    }

    let doc = xml::parse(&xml)?;

    let mut out = String::new();
    for para in doc
        .root_element()
        .descendants()
        .filter(|n| n.tag_name().name() == "p")
    {
        let p_pr = para.children().find(|n| n.tag_name().name() == "pPr");
        let style = p_pr.and_then(|pr| {
            pr.children()
                .find(|n| n.tag_name().name() == "pStyle")
                .and_then(|s| attr_local(s, "val"))
        });
        // Only shallow outline levels (0-2 → #/##/###) are treated as real
        // headings. Word assigns deeper outline slots (3-9) to body styles
        // like "exercise title"/"question", so those stay plain.
        let level = style
            .and_then(|s| {
                outline
                    .get(s)
                    .copied()
                    .map(|l| l + 1)
                    .or_else(|| heading_level(s))
            })
            .filter(|l| *l <= 3);
        let is_list = p_pr
            .map(|pr| pr.children().any(|n| n.tag_name().name() == "numPr"))
            .unwrap_or(false);

        // Collect runs as (bold, italic, text), coalescing consecutive runs
        // that share the same emphasis. Word splits one styled phrase across
        // many runs; wrapping each separately would emit `**a****b**` which
        // markdown can't parse.
        let mut segs: Vec<(bool, bool, String)> = Vec::new();
        for run in para.children().filter(|n| n.tag_name().name() == "r") {
            let r_pr = run.children().find(|n| n.tag_name().name() == "rPr");
            let bold = r_pr
                .map(|pr| pr.children().any(|n| n.tag_name().name() == "b"))
                .unwrap_or(false);
            let italic = r_pr
                .map(|pr| pr.children().any(|n| n.tag_name().name() == "i"))
                .unwrap_or(false);
            let text: String = run
                .descendants()
                .filter(|n| n.tag_name().name() == "t")
                .flat_map(|t| t.descendants())
                .filter(|n| n.is_text())
                .filter_map(|n| n.text())
                .collect();
            if text.is_empty() {
                continue;
            }
            match segs.last_mut() {
                Some((b, i, t)) if *b == bold && *i == italic => t.push_str(&text),
                _ => segs.push((bold, italic, text)),
            }
        }
        let mut body = String::new();
        for (bold, italic, text) in &segs {
            body.push_str(&wrap_emphasis(&escape_md(text), *bold, *italic));
        }

        if body.trim().is_empty() {
            // Preserve paragraph breaks even for empty paragraphs.
            out.push('\n');
            continue;
        }

        match level {
            Some(n) => {
                out.push_str(&"#".repeat(n));
                out.push(' ');
                out.push_str(body.trim());
            }
            None if is_list => {
                out.push_str("- ");
                out.push_str(body.trim());
            }
            None => out.push_str(&escape_leading_block(body.trim())),
        }
        // Blank line between blocks so react-markdown treats them separately.
        out.push_str("\n\n");
    }

    Ok(normalize_md(&out))
}

// ODF (odt/odp) → Markdown. Headings are explicit (<text:h outline-level=N>);
// bold/italic come from automatic styles resolved by name.
fn extract_odf_markdown(path: &str, budget: &mut Budget) -> Result<String, String> {
    let mut zip = pkg::open_zip(path)?;
    let xml = pkg::read_entry(&mut zip, "content.xml", budget)?
        .ok_or_else(|| "odf: missing content.xml".to_string())?;
    let doc = xml::parse(&xml)?;
    let root = doc.root_element();

    // style-name → (bold, italic), built from <style:style> text-properties.
    let mut styles: std::collections::HashMap<String, (bool, bool)> =
        std::collections::HashMap::new();
    for style in root.descendants().filter(|n| n.tag_name().name() == "style") {
        let Some(name) = attr_local(style, "name") else {
            continue;
        };
        let props = style
            .children()
            .find(|n| n.tag_name().name() == "text-properties");
        let bold = props
            .and_then(|p| attr_local(p, "font-weight"))
            .map(|w| w == "bold")
            .unwrap_or(false);
        let italic = props
            .and_then(|p| attr_local(p, "font-style"))
            .map(|s| s == "italic")
            .unwrap_or(false);
        if bold || italic {
            styles.insert(name.to_string(), (bold, italic));
        }
    }

    let in_list = |node: roxmltree::Node| -> bool {
        node.ancestors().any(|a| a.tag_name().name() == "list-item")
    };

    let mut out = String::new();
    for para in root
        .descendants()
        .filter(|n| matches!(n.tag_name().name(), "p" | "h"))
    {
        let is_heading = para.tag_name().name() == "h";
        let level = if is_heading {
            attr_local(para, "outline-level")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, 6)
        } else {
            0
        };

        // Gather text from this block, applying span styles for emphasis.
        let mut segs: Vec<(bool, bool, String)> = Vec::new();
        odf_collect(para, &styles, (false, false), &mut segs);
        let mut body = String::new();
        for (bold, italic, text) in &segs {
            body.push_str(&wrap_emphasis(&escape_md(text), *bold, *italic));
        }

        if body.trim().is_empty() {
            out.push('\n');
            continue;
        }

        if level > 0 {
            out.push_str(&"#".repeat(level));
            out.push(' ');
            out.push_str(body.trim());
        } else if in_list(para) {
            out.push_str("- ");
            out.push_str(body.trim());
        } else {
            out.push_str(&escape_leading_block(body.trim()));
        }
        out.push_str("\n\n");
    }

    Ok(normalize_md(&out))
}

// Walk a text:p / text:h into (bold, italic, text) segments, inheriting emphasis
// from enclosing styled <text:span>s and coalescing adjacent same-style text so
// we never emit `**a****b**`.
fn odf_collect(
    node: roxmltree::Node,
    styles: &std::collections::HashMap<String, (bool, bool)>,
    cur: (bool, bool),
    out: &mut Vec<(bool, bool, String)>,
) {
    if node.is_text() {
        if let Some(t) = node.text() {
            match out.last_mut() {
                Some((b, i, s)) if *b == cur.0 && *i == cur.1 => s.push_str(t),
                _ => out.push((cur.0, cur.1, t.to_string())),
            }
        }
        return;
    }
    // A styled span turns emphasis on for its subtree.
    let mut style = cur;
    if node.tag_name().name() == "span" {
        if let Some((b, i)) = attr_local(node, "style-name").and_then(|n| styles.get(n)) {
            style = (cur.0 || *b, cur.1 || *i);
        }
    }
    // Skip nested paragraphs/headings (each is emitted as its own block).
    for child in node.children() {
        if matches!(child.tag_name().name(), "p" | "h") {
            continue;
        }
        odf_collect(child, styles, style, out);
    }
}

// Collapse runs of 3+ newlines down to the blank-line separator markdown wants.
fn normalize_md(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut newline_run = 0usize;
    for ch in s.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                result.push('\n');
            }
        } else {
            newline_run = 0;
            result.push(ch);
        }
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_md_collapses_newline_runs() {
        assert_eq!(normalize_md("café\n\n\n\nnaïve"), "café\n\nnaïve");
    }

    #[test]
    fn escape_leading_block_neutralizes_block_starters() {
        assert_eq!(escape_leading_block("1. Widget"), "1\\. Widget");
        assert_eq!(escape_leading_block("- Widget"), "\\- Widget");
        assert_eq!(escape_leading_block("# Widget"), "\\# Widget");
        assert_eq!(escape_leading_block("café"), "café");
    }
}
