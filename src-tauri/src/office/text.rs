//! Flat text extraction (for content indexing).

use super::pkg::{self, Budget};
use super::xml;
use std::path::Path;

pub fn extract_office_text(path: &str) -> Result<String, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut budget = Budget::new();
    match ext.as_str() {
        "docx" => extract_docx(path, &mut budget),
        "pptx" => extract_pptx(path, &mut budget),
        "xlsx" => extract_xlsx(path, &mut budget),
        "odt" | "ods" | "odp" => extract_odf_text(path, &mut budget),
        other => Err(format!("unsupported office extension: {other}")),
    }
}

fn extract_docx(path: &str, budget: &mut Budget) -> Result<String, String> {
    let mut zip = pkg::open_zip(path)?;
    let xml = pkg::read_entry(&mut zip, "word/document.xml", budget)?
        .ok_or_else(|| "docx: missing word/document.xml".to_string())?;
    xml::xml_text(&xml, &["p"], &["t"])
}

fn extract_pptx(path: &str, budget: &mut Budget) -> Result<String, String> {
    let mut zip = pkg::open_zip(path)?;

    // Collect slide names first (while zip is the only borrow).
    let slide_names: Vec<String> = pkg::list_parts(&mut zip, |name| {
        name.starts_with("ppt/slides/slide") && name.ends_with(".xml")
    });

    let mut slides = slide_names;
    slides.sort_by(|a, b| xml::natural_cmp(a, b));

    let mut out = String::new();
    for name in slides {
        let xml = match pkg::read_entry(&mut zip, &name, budget) {
            Ok(Some(x)) => x,
            Ok(None) => continue,
            // Budget exhausted: stop accumulating rather than failing the whole
            // deck, so the slides read so far are still indexed. (Pre-split this
            // was `total += xml.len(); if total > MAX_TOTAL_BYTES { break }`.)
            Err(e) if e == pkg::BUDGET_EXCEEDED => break,
            Err(e) => return Err(e),
        };
        let text = xml::xml_text(&xml, &["p"], &["t"])?;
        if !text.trim().is_empty() {
            out.push_str(&text);
            out.push('\n');
        }
    }
    Ok(xml::normalize(&out))
}

fn extract_xlsx(path: &str, budget: &mut Budget) -> Result<String, String> {
    let mut zip = pkg::open_zip(path)?;
    match pkg::read_entry(&mut zip, "xl/sharedStrings.xml", budget)? {
        Some(xml) => xml::xml_text(&xml, &["si"], &["t"]),
        None => Ok(String::new()),
    }
}

fn extract_odf_text(path: &str, budget: &mut Budget) -> Result<String, String> {
    let mut zip = pkg::open_zip(path)?;
    let xml = pkg::read_entry(&mut zip, "content.xml", budget)?
        .ok_or_else(|| "odf: missing content.xml".to_string())?;
    let doc = xml::parse(&xml)?;
    let mut out = String::new();
    xml::odf_walk(doc.root_element(), &mut out);
    Ok(xml::normalize(&out))
}
