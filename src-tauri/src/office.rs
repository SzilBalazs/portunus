use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

// Guards against zip bombs: reject inflated entries above these limits.
const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

// Preview: max rows/columns extracted from a spreadsheet.
const MAX_ROWS: usize = 100;
const MAX_COLS: usize = 50;

pub const OFFICE_EXTENSIONS: &[&str] = &["docx", "pptx", "xlsx", "odt", "ods", "odp"];

pub fn is_office_ext(ext: &str) -> bool {
    OFFICE_EXTENSIONS.contains(&ext)
}

// ── zip helpers ───────────────────────────────────────────────────────────────

type Zip = ZipArchive<std::io::BufReader<std::fs::File>>;

fn open_zip(path: &str) -> Result<Zip, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| e.to_string())
}

fn read_entry(zip: &mut Zip, name: &str) -> Result<Option<String>, String> {
    let mut entry = match zip.by_name(name) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if entry.size() > MAX_ENTRY_BYTES {
        return Err(format!("entry {} too large: {} bytes", name, entry.size()));
    }
    let mut buf = String::new();
    entry.read_to_string(&mut buf).map_err(|e| e.to_string())?;
    Ok(Some(buf))
}

// ── XML text helper (OOXML: text in <t> elements) ─────────────────────────────

fn xml_text(xml: &str, para_tags: &[&str], text_tags: &[&str]) -> Result<String, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let mut out = String::new();
    xml_walk(doc.root_element(), para_tags, text_tags, &mut out);
    Ok(normalize(&out))
}

fn xml_walk(
    node: roxmltree::Node,
    para_tags: &[&str],
    text_tags: &[&str],
    out: &mut String,
) {
    let local = node.tag_name().name();
    if text_tags.contains(&local) {
        // Slurp all descendant text (handles split runs and whitespace-preserve).
        for d in node.descendants() {
            if d.is_text() {
                if let Some(t) = d.text() {
                    out.push_str(t);
                }
            }
        }
        return;
    }
    for child in node.children() {
        xml_walk(child, para_tags, text_tags, out);
    }
    if para_tags.contains(&local) {
        out.push('\n');
    }
}

// ODF text often sits as a direct text node of text:p / text:span rather than
// inside a dedicated <t>; give it a dedicated walker.
fn odf_walk(node: roxmltree::Node, out: &mut String) {
    if node.is_text() {
        if let Some(t) = node.text() {
            out.push_str(t);
        }
        return;
    }
    for child in node.children() {
        odf_walk(child, out);
    }
    let local = node.tag_name().name();
    if local == "p" || local == "h" {
        out.push('\n');
    }
}

fn normalize(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut blank_run = 0usize;
    for line in s.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                result.push('\n');
            }
        } else {
            blank_run = 0;
            result.push_str(line);
            result.push('\n');
        }
    }
    result.trim_end().to_string()
}

// ── natural sort for pptx slide filenames ─────────────────────────────────────

fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let mut ai = a.bytes().peekable();
    let mut bi = b.bytes().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = take_num(&mut ai);
                let nb = take_num(&mut bi);
                match na.cmp(&nb) {
                    Ordering::Equal => continue,
                    ord => return ord,
                }
            }
            (Some(x), Some(y)) => match x.cmp(&y) {
                Ordering::Equal => {
                    ai.next();
                    bi.next();
                }
                ord => return ord,
            },
        }
    }
}

fn take_num<I: Iterator<Item = u8>>(it: &mut std::iter::Peekable<I>) -> u64 {
    let mut n: u64 = 0;
    while let Some(&d) = it.peek() {
        if !d.is_ascii_digit() {
            break;
        }
        n = n.saturating_mul(10).saturating_add((d - b'0') as u64);
        it.next();
    }
    n
}

// ── flat text extraction (for content indexing) ───────────────────────────────

pub fn extract_office_text(path: &str) -> Result<String, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "docx" => extract_docx(path),
        "pptx" => extract_pptx(path),
        "xlsx" => extract_xlsx(path),
        "odt" | "ods" | "odp" => extract_odf_text(path),
        other => Err(format!("unsupported office extension: {other}")),
    }
}

fn extract_docx(path: &str) -> Result<String, String> {
    let mut zip = open_zip(path)?;
    let xml = read_entry(&mut zip, "word/document.xml")?
        .ok_or_else(|| "docx: missing word/document.xml".to_string())?;
    xml_text(&xml, &["p"], &["t"])
}

fn extract_pptx(path: &str) -> Result<String, String> {
    let mut zip = open_zip(path)?;

    // Collect slide names first (while zip is the only borrow).
    let slide_names: Vec<String> = {
        let len = zip.len();
        (0..len)
            .filter_map(|i| {
                let f = zip.by_index(i).ok()?;
                let name = f.name();
                if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
                    Some(name.to_string())
                } else {
                    None
                }
            })
            .collect()
    };

    let mut slides = slide_names;
    slides.sort_by(|a, b| natural_cmp(a, b));

    let mut out = String::new();
    let mut total: u64 = 0;
    for name in slides {
        let Some(xml) = read_entry(&mut zip, &name)? else {
            continue;
        };
        total += xml.len() as u64;
        if total > MAX_TOTAL_BYTES {
            break;
        }
        let text = xml_text(&xml, &["p"], &["t"])?;
        if !text.trim().is_empty() {
            out.push_str(&text);
            out.push('\n');
        }
    }
    Ok(normalize(&out))
}

fn extract_xlsx(path: &str) -> Result<String, String> {
    let mut zip = open_zip(path)?;
    match read_entry(&mut zip, "xl/sharedStrings.xml")? {
        Some(xml) => xml_text(&xml, &["si"], &["t"]),
        None => Ok(String::new()),
    }
}

fn extract_odf_text(path: &str) -> Result<String, String> {
    let mut zip = open_zip(path)?;
    let xml = read_entry(&mut zip, "content.xml")?
        .ok_or_else(|| "odf: missing content.xml".to_string())?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
    let mut out = String::new();
    odf_walk(doc.root_element(), &mut out);
    Ok(normalize(&out))
}

// ── markdown extraction (for preview) ─────────────────────────────────────────

// Like extract_office_text, but preserves headings / bold / italic / lists as
// Markdown so the preview can render formatting. Plain text remains the fallback
// for formats we don't enrich (pptx).
pub fn extract_office_markdown(path: &str) -> Result<String, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "docx" => extract_docx_markdown(path),
        "odt" | "odp" => extract_odf_markdown(path),
        _ => extract_office_text(path),
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

// Find an attribute by local name, ignoring its namespace prefix. roxmltree's
// `attribute("val")` matches only the empty namespace, so namespaced attrs like
// `w:val` / `text:outline-level` must be located this way.
fn attr_local<'a>(node: roxmltree::Node<'a, 'a>, local: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name() == local)
        .map(|a| a.value())
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

fn extract_docx_markdown(path: &str) -> Result<String, String> {
    let mut zip = open_zip(path)?;
    // styleId → outline level, used to detect headings independent of the UI
    // language (style names are localized, e.g. "Cmsor1" = Heading 1 in HU).
    let styles_xml = read_entry(&mut zip, "word/styles.xml")?;
    let xml = read_entry(&mut zip, "word/document.xml")?
        .ok_or_else(|| "docx: missing word/document.xml".to_string())?;

    let mut outline: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    if let Some(sx) = &styles_xml {
        if let Ok(sdoc) = roxmltree::Document::parse(sx) {
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

    let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;

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
fn extract_odf_markdown(path: &str) -> Result<String, String> {
    let mut zip = open_zip(path)?;
    let xml = read_entry(&mut zip, "content.xml")?
        .ok_or_else(|| "odf: missing content.xml".to_string())?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
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

// ── spreadsheet grid extraction (for preview) ─────────────────────────────────

// Returns a 2-D grid (rows × cols) of cell strings, capped at MAX_ROWS × MAX_COLS.
pub fn extract_spreadsheet_grid(path: &str) -> Result<Vec<Vec<String>>, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "xlsx" => extract_xlsx_grid(path),
        "ods" => extract_ods_grid(path),
        other => Err(format!("not a spreadsheet: {other}")),
    }
}

// ── xlsx grid ────────────────────────────────────────────────────────────────

// Convert Excel column letters to a 0-based index ("A"→0, "Z"→25, "AA"→26…).
fn col_letter_to_index(col: &str) -> Option<usize> {
    let mut idx: usize = 0;
    for ch in col.bytes() {
        if !ch.is_ascii_alphabetic() {
            break;
        }
        idx = idx * 26 + (ch.to_ascii_uppercase() - b'A') as usize + 1;
    }
    if idx == 0 {
        None
    } else {
        Some(idx - 1)
    }
}

// Split a cell reference like "AB12" into the letter prefix and digit suffix.
fn split_cell_ref(r: &str) -> (&str, &str) {
    let split = r.find(|c: char| c.is_ascii_digit()).unwrap_or(r.len());
    (&r[..split], &r[split..])
}

fn extract_xlsx_grid(path: &str) -> Result<Vec<Vec<String>>, String> {
    let mut zip = open_zip(path)?;

    // Build shared string pool.
    let pool: Vec<String> = match read_entry(&mut zip, "xl/sharedStrings.xml")? {
        Some(xml) => {
            let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
            doc.root_element()
                .descendants()
                .filter(|n| n.tag_name().name() == "si")
                .map(|si| {
                    si.descendants()
                        .filter(|n| n.tag_name().name() == "t")
                        .filter_map(|t| {
                            t.descendants()
                                .find(|n| n.is_text())
                                .and_then(|n| n.text())
                        })
                        .collect::<String>()
                })
                .collect()
        }
        None => Vec::new(),
    };

    let xml = match read_entry(&mut zip, "xl/worksheets/sheet1.xml")? {
        Some(x) => x,
        None => return Ok(Vec::new()),
    };
    let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;

    let mut grid: Vec<Vec<String>> = Vec::new();

    for row_node in doc
        .root_element()
        .descendants()
        .filter(|n| n.tag_name().name() == "row")
        .take(MAX_ROWS)
    {
        let mut row: Vec<(usize, String)> = Vec::new();
        for cell in row_node
            .children()
            .filter(|n| n.tag_name().name() == "c")
        {
            let r_attr = cell.attribute("r").unwrap_or("");
            let (col_str, _) = split_cell_ref(r_attr);
            let Some(col_idx) = col_letter_to_index(col_str) else {
                continue;
            };
            if col_idx >= MAX_COLS {
                continue;
            }
            let cell_type = cell.attribute("t").unwrap_or("");
            let value: String = match cell_type {
                "s" => {
                    // shared string index
                    let idx: usize = cell
                        .descendants()
                        .find(|n| n.tag_name().name() == "v")
                        .and_then(|v| v.text())
                        .and_then(|t| t.parse().ok())
                        .unwrap_or(0);
                    pool.get(idx).cloned().unwrap_or_default()
                }
                "inlineStr" => cell
                    .descendants()
                    .find(|n| n.tag_name().name() == "t")
                    .and_then(|t| t.text())
                    .unwrap_or("")
                    .to_string(),
                _ => cell
                    .descendants()
                    .find(|n| n.tag_name().name() == "v")
                    .and_then(|v| v.text())
                    .unwrap_or("")
                    .to_string(),
            };
            let value = truncate_cell(value);
            row.push((col_idx, value));
        }
        // Expand sparse row to a dense vec filling gaps with "".
        let max_col = row.iter().map(|(c, _)| c + 1).max().unwrap_or(0);
        let mut dense = vec![String::new(); max_col];
        for (col, val) in row {
            dense[col] = val;
        }
        grid.push(dense);
    }

    Ok(grid)
}

// ── ods grid ─────────────────────────────────────────────────────────────────

fn extract_ods_grid(path: &str) -> Result<Vec<Vec<String>>, String> {
    let mut zip = open_zip(path)?;
    let xml = read_entry(&mut zip, "content.xml")?
        .ok_or_else(|| "ods: missing content.xml".to_string())?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;

    let mut grid: Vec<Vec<String>> = Vec::new();

    // Find the first <table:table> element.
    let table = match doc
        .root_element()
        .descendants()
        .find(|n| n.tag_name().name() == "table")
    {
        Some(t) => t,
        None => return Ok(Vec::new()),
    };

    for row_node in table
        .children()
        .filter(|n| n.tag_name().name() == "table-row")
    {
        let repeat_rows = row_node
            .attribute(("urn:oasis:names:tc:opendocument:xmlns:table:1.0", "number-rows-repeated"))
            .or_else(|| row_node.attributes().find(|a| a.name() == "number-rows-repeated").map(|a| a.value()))
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1)
            // Cap to avoid expanding 65536-repeat trailing filler rows.
            .min(MAX_ROWS.saturating_sub(grid.len()).max(1));

        let mut row: Vec<String> = Vec::new();
        for cell in row_node
            .children()
            .filter(|n| n.tag_name().name() == "table-cell")
        {
            let repeat_cols = cell
                .attribute(("urn:oasis:names:tc:opendocument:xmlns:table:1.0", "number-columns-repeated"))
                .or_else(|| cell.attributes().find(|a| a.name() == "number-columns-repeated").map(|a| a.value()))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1)
                .min(MAX_COLS.saturating_sub(row.len()).max(1));

            // Collect text from all text:p children.
            let text: String = cell
                .descendants()
                .filter(|n| n.tag_name().name() == "p")
                .map(|p| {
                    p.descendants()
                        .filter(|n| n.is_text())
                        .filter_map(|n| n.text())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join(" ");
            let text = truncate_cell(text);

            for _ in 0..repeat_cols {
                if row.len() >= MAX_COLS {
                    break;
                }
                row.push(text.clone());
            }
        }

        for _ in 0..repeat_rows {
            if grid.len() >= MAX_ROWS {
                break;
            }
            grid.push(row.clone());
        }
    }

    Ok(grid)
}

fn truncate_cell(mut s: String) -> String {
    const MAX_CELL: usize = 200;
    if s.len() > MAX_CELL {
        // Walk back to the nearest char boundary so we don't split a multibyte codepoint.
        let mut cut = MAX_CELL;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    s
}
