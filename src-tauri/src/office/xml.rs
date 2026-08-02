//! Shared XML plumbing for the office extractors: the hardened parser entry
//! point plus the generic text walkers and helpers used by every format.

// Single parser entry point for all document XML. Document XML is untrusted
// input, so DTDs stay disabled: an external entity (`<!ENTITY x SYSTEM
// "file:///etc/passwd">`) would turn a preview into a local-file read (XXE),
// and nested internal entities are the billion-laughs expansion. `nodes_limit`
// bounds peak memory on a hostile-but-well-formed part (MAX_ENTRY_BYTES only
// bounds the input string, not the node arena).
pub fn parse(xml: &str) -> Result<roxmltree::Document<'_>, String> {
    roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 2_000_000,
        },
    )
    .map_err(|e| e.to_string())
}

// ── XML text helper (OOXML: text in <t> elements) ─────────────────────────────

pub fn xml_text(xml: &str, para_tags: &[&str], text_tags: &[&str]) -> Result<String, String> {
    let doc = parse(xml)?;
    let mut out = String::new();
    xml_walk(doc.root_element(), para_tags, text_tags, &mut out);
    Ok(normalize(&out))
}

pub fn xml_walk(
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
pub fn odf_walk(node: roxmltree::Node, out: &mut String) {
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

pub fn normalize(s: &str) -> String {
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

// Find an attribute by local name, ignoring its namespace prefix. roxmltree's
// `attribute("val")` matches only the empty namespace, so namespaced attrs like
// `w:val` / `text:outline-level` must be located this way.
pub fn attr_local<'a>(node: roxmltree::Node<'a, 'a>, local: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name() == local)
        .map(|a| a.value())
}

// ── natural sort for pptx slide filenames ─────────────────────────────────────

pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
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

pub fn take_num<I: Iterator<Item = u8>>(it: &mut std::iter::Peekable<I>) -> u64 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_doctype_entities() {
        // XXE: a DTD-declared external entity must never be resolved (which
        // would read the referenced file into the document text).
        let xml = r#"<!DOCTYPE foo [<!ENTITY x SYSTEM "file:///etc/passwd">]><r>&x;</r>"#;
        let err = parse(xml).expect_err("DTD must be rejected");
        assert!(!err.is_empty());
    }

    #[test]
    fn parse_rejects_billion_laughs() {
        let xml = r#"<!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;">]><r>&lol2;</r>"#;
        assert!(parse(xml).is_err());
    }

    #[test]
    fn parse_accepts_plain_document() {
        let doc = parse("<r><p>café</p></r>").expect("plain XML parses");
        assert_eq!(doc.root_element().tag_name().name(), "r");
    }

    #[test]
    fn natural_cmp_orders_slide_numbers_numerically() {
        assert_eq!(
            natural_cmp("ppt/slides/slide2.xml", "ppt/slides/slide10.xml"),
            std::cmp::Ordering::Less
        );
        assert_eq!(natural_cmp("slide10.xml", "slide2.xml"), std::cmp::Ordering::Greater);
        assert_eq!(natural_cmp("slide2.xml", "slide2.xml"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn normalize_collapses_blank_line_runs() {
        assert_eq!(normalize("a\n\n\n\nb"), "a\n\nb");
        assert_eq!(normalize("café   \n\n\n naïve"), "café\n\n naïve");
    }
}
