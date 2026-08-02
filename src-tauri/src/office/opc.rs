//! Open Packaging Conventions plumbing shared by the OOXML formats:
//! `_rels/*.rels` relationships, package-path resolution, and content-type
//! lookup via `[Content_Types].xml`.
//!
//! Nothing here is wired into the text/markdown extractors yet; the HTML
//! renderers in the later stages are the consumers.
#![allow(dead_code)]

use super::xml;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    /// Raw `Target` attribute, unresolved. Use `resolve_target` for internal
    /// targets; external ones are URIs and must not be treated as paths.
    pub target: String,
    /// The `Type` attribute (relationship kind URI).
    pub kind: String,
    /// `TargetMode="External"` — target is a URI, not a package part.
    pub external: bool,
}

/// Parse a `_rels/*.rels` part into rId → relationship.
pub fn parse_rels(rels_xml: &str) -> Result<HashMap<String, Relationship>, String> {
    let doc = xml::parse(rels_xml)?;
    let mut map = HashMap::new();
    for rel in doc
        .root_element()
        .children()
        .filter(|n| n.tag_name().name() == "Relationship")
    {
        let Some(id) = xml::attr_local(rel, "Id") else {
            continue;
        };
        let Some(target) = xml::attr_local(rel, "Target") else {
            continue;
        };
        let external = xml::attr_local(rel, "TargetMode")
            .map(|m| m.eq_ignore_ascii_case("external"))
            .unwrap_or(false);
        map.insert(
            id.to_string(),
            Relationship {
                target: target.to_string(),
                kind: xml::attr_local(rel, "Type").unwrap_or("").to_string(),
                external,
            },
        );
    }
    Ok(map)
}

/// The `_rels` part path holding the relationships of `part`
/// ("ppt/slides/slide1.xml" → "ppt/slides/_rels/slide1.xml.rels").
pub fn rels_path_for(part: &str) -> String {
    match part.rfind('/') {
        Some(i) => format!("{}/_rels/{}.rels", &part[..i], &part[i + 1..]),
        None => format!("_rels/{part}.rels"),
    }
}

/// Resolve a relationship `Target` against the directory of the part that owns
/// it, producing a normalized package path with no leading slash. A leading `/`
/// on the target means "from the package root". Returns `None` when the target
/// is empty or walks outside the package root — a part path that escapes the
/// zip is never legal and must not reach the filesystem.
pub fn resolve_target(owner_part: &str, target: &str) -> Option<String> {
    if target.is_empty() {
        return None;
    }
    let mut stack: Vec<&str> = Vec::new();
    if !target.starts_with('/') {
        let dir = match owner_part.rfind('/') {
            Some(i) => &owner_part[..i],
            None => "",
        };
        for seg in dir.split('/') {
            if seg.is_empty() || seg == "." {
                continue;
            }
            if seg == ".." {
                stack.pop()?;
                continue;
            }
            stack.push(seg);
        }
    }
    for seg in target.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            stack.pop()?;
            continue;
        }
        stack.push(seg);
    }
    if stack.is_empty() {
        return None;
    }
    Some(stack.join("/"))
}

/// `[Content_Types].xml`: extension defaults plus per-part overrides.
#[derive(Debug, Default)]
pub struct ContentTypes {
    defaults: HashMap<String, String>,
    overrides: HashMap<String, String>,
}

impl ContentTypes {
    pub fn parse(types_xml: &str) -> Result<Self, String> {
        let doc = xml::parse(types_xml)?;
        let mut ct = ContentTypes::default();
        for node in doc.root_element().children() {
            let Some(content_type) = xml::attr_local(node, "ContentType") else {
                continue;
            };
            match node.tag_name().name() {
                "Default" => {
                    if let Some(ext) = xml::attr_local(node, "Extension") {
                        ct.defaults
                            .insert(ext.to_ascii_lowercase(), content_type.to_string());
                    }
                }
                "Override" => {
                    if let Some(part) = xml::attr_local(node, "PartName") {
                        // PartName is always package-absolute; store it without
                        // the leading slash so lookups take package paths.
                        ct.overrides.insert(
                            part.trim_start_matches('/').to_string(),
                            content_type.to_string(),
                        );
                    }
                }
                _ => {}
            }
        }
        Ok(ct)
    }

    /// Content type of `part` (a package path, no leading slash): the Override
    /// wins, else the Default for the part's extension.
    pub fn for_part(&self, part: &str) -> Option<&str> {
        let part = part.trim_start_matches('/');
        if let Some(t) = self.overrides.get(part) {
            return Some(t.as_str());
        }
        let ext = part.rsplit_once('.')?.1.to_ascii_lowercase();
        self.defaults.get(&ext).map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_target_against_owner_dir() {
        assert_eq!(
            resolve_target("ppt/slides/slide1.xml", "../media/image1.png").as_deref(),
            Some("ppt/media/image1.png")
        );
        assert_eq!(
            resolve_target("word/document.xml", "media/image1.png").as_deref(),
            Some("word/media/image1.png")
        );
        assert_eq!(
            resolve_target("ppt/slides/slide1.xml", "./slide2.xml").as_deref(),
            Some("ppt/slides/slide2.xml")
        );
    }

    #[test]
    fn resolves_absolute_target_from_package_root() {
        assert_eq!(
            resolve_target("ppt/slides/slide1.xml", "/ppt/media/image1.png").as_deref(),
            Some("ppt/media/image1.png")
        );
    }

    #[test]
    fn rejects_traversal_outside_package_root() {
        assert_eq!(resolve_target("word/document.xml", "../../etc/passwd"), None);
        assert_eq!(resolve_target("document.xml", "../secret"), None);
        assert_eq!(
            resolve_target("ppt/slides/slide1.xml", "../../../../etc/passwd"),
            None
        );
        assert_eq!(resolve_target("word/document.xml", ""), None);
        assert_eq!(resolve_target("word/document.xml", "/"), None);
    }

    #[test]
    fn rels_path_mirrors_part_path() {
        assert_eq!(
            rels_path_for("ppt/slides/slide1.xml"),
            "ppt/slides/_rels/slide1.xml.rels"
        );
        assert_eq!(rels_path_for("word/document.xml"), "word/_rels/document.xml.rels");
        assert_eq!(rels_path_for("content.xml"), "_rels/content.xml.rels");
    }

    #[test]
    fn parses_relationships_including_external() {
        let xml = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.org/" TargetMode="External"/>
</Relationships>"#;
        let rels = parse_rels(xml).expect("rels parse");
        assert_eq!(rels.len(), 2);
        let img = &rels["rId1"];
        assert_eq!(img.target, "../media/image1.png");
        assert!(!img.external);
        assert!(img.kind.ends_with("/image"));
        let link = &rels["rId2"];
        assert!(link.external);
        assert_eq!(link.target, "https://example.org/");
        assert_eq!(
            resolve_target("ppt/slides/slide1.xml", &img.target).as_deref(),
            Some("ppt/media/image1.png")
        );
    }

    #[test]
    fn content_type_override_beats_default() {
        let xml = r#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="PNG" ContentType="image/png"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
        let ct = ContentTypes::parse(xml).expect("content types parse");
        assert_eq!(ct.for_part("word/media/image1.png"), Some("image/png"));
        assert_eq!(ct.for_part("word/styles.xml"), Some("application/xml"));
        assert_eq!(
            ct.for_part("word/document.xml"),
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml")
        );
        assert_eq!(ct.for_part("word/noext"), None);
    }
}
