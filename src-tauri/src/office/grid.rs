//! Spreadsheet grid extraction for the preview.
//!
//! INTERIM: this module is removed once the HTML renderers land in a later
//! stage. Don't invest in it — it is here unchanged so the split stays a pure
//! move.

use super::pkg::{self, Budget};
use super::xml;
use std::path::Path;

// Preview: max rows/columns extracted from a spreadsheet.
const MAX_ROWS: usize = 100;
const MAX_COLS: usize = 50;

// Returns a 2-D grid (rows × cols) of cell strings, capped at MAX_ROWS × MAX_COLS.
pub fn extract_spreadsheet_grid(path: &str) -> Result<Vec<Vec<String>>, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut budget = Budget::new();
    match ext.as_str() {
        "xlsx" => extract_xlsx_grid(path, &mut budget),
        "ods" => extract_ods_grid(path, &mut budget),
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

fn extract_xlsx_grid(path: &str, budget: &mut Budget) -> Result<Vec<Vec<String>>, String> {
    let mut zip = pkg::open_zip(path)?;

    // Build shared string pool.
    let pool: Vec<String> = match pkg::read_entry(&mut zip, "xl/sharedStrings.xml", budget)? {
        Some(xml) => {
            let doc = xml::parse(&xml)?;
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

    let xml = match pkg::read_entry(&mut zip, "xl/worksheets/sheet1.xml", budget)? {
        Some(x) => x,
        None => return Ok(Vec::new()),
    };
    let doc = xml::parse(&xml)?;

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

fn extract_ods_grid(path: &str, budget: &mut Budget) -> Result<Vec<Vec<String>>, String> {
    let mut zip = pkg::open_zip(path)?;
    let xml = pkg::read_entry(&mut zip, "content.xml", budget)?
        .ok_or_else(|| "ods: missing content.xml".to_string())?;
    let doc = xml::parse(&xml)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_letters_map_to_zero_based_indices() {
        assert_eq!(col_letter_to_index("A"), Some(0));
        assert_eq!(col_letter_to_index("Z"), Some(25));
        assert_eq!(col_letter_to_index("AA"), Some(26));
        assert_eq!(col_letter_to_index("XFD"), Some(16383));
        assert_eq!(col_letter_to_index(""), None);
        assert_eq!(col_letter_to_index("12"), None);
    }

    #[test]
    fn cell_ref_splits_letters_from_digits() {
        assert_eq!(split_cell_ref("AB12"), ("AB", "12"));
        assert_eq!(split_cell_ref("A1"), ("A", "1"));
        assert_eq!(split_cell_ref("Sheet"), ("Sheet", ""));
        assert_eq!(split_cell_ref(""), ("", ""));
    }

    #[test]
    fn truncate_cell_keeps_char_boundaries() {
        let long = "café".repeat(200);
        let cut = truncate_cell(long);
        assert!(cut.ends_with('…'));
        // `café` is 5 bytes, so the 200-byte cut lands mid-`é` and walks back.
        assert!(cut.len() <= 200 + '…'.len_utf8());
    }
}
