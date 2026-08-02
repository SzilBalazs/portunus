//! One worksheet → an HTML `<table>` with a row/column gutter, resolved cell
//! formats as classes, merged-cell spans, frozen panes and the drawing overlay.

use super::super::drawingml::geom::Xf;
use super::super::html::{attr, attrs, emu_to_px, pt_to_px, Writer};
use super::super::media::{self, Media};
use super::super::numfmt::Format;
use super::super::{opc, pkg, xml};
use super::styles::{bool_attr, f32_attr, u32_attr};
use super::{col_letter, col_letter_to_index, note, split_cell_ref, Ctx, SheetRef};
use std::collections::BTreeSet;

/// Width of the row-number gutter, and height of the column-letter header.
const ROW_HDR_PX: f32 = 46.0;
const COL_HDR_PX: f32 = 21.0;

/// Max digit width of the default font (Calibri 11) in px, plus Excel's fixed
/// cell padding. A stored column `width` is a count of those digits, so
/// `px = width * 7 + 5` — which reproduces the familiar default: 8.43 chars is
/// 64px. It is an approximation because the real max-digit-width depends on the
/// workbook's default font, which the preview does not measure.
const MDW_PX: f32 = 7.0;
const COL_PAD_PX: f32 = 5.0;
const DEFAULT_COL_CHARS: f32 = 8.43;
const DEFAULT_ROW_PT: f32 = 15.0;

/// A column narrower than this is still given a sliver, so a hidden-by-zero-width
/// column does not collapse the gutter alignment.
const MIN_COL_PX: f32 = 2.0;
const MAX_COL_PX: f32 = 2000.0;
const MAX_ROW_PX: f32 = 1000.0;

/// Sticky offsets are generated per frozen row/column, so the count is bounded.
/// Panes deeper than this are not sticky (and nothing else about them changes).
const MAX_FROZEN: usize = 16;

const MAX_MERGES: usize = 8192;
const MAX_COL_DEFS: usize = 4096;
const MAX_DRAWINGS: usize = 200;

/// Longest rendered cell string. Excel's own limit is 32767 characters, which a
/// grid cell cannot show anyway.
const MAX_CELL_CHARS: usize = 512;

/// Structural stylesheet.
///
/// Every selector here is at most one type + one class, so the per-`xf` rules
/// (`td.xfN`, appended after this block) tie on specificity and win on source
/// order. A `.xl-doc table.xl-sheet td` style rule would quietly beat every
/// document-authored border and fill.
const BASE_CSS: &str = "\
.xl-doc{font-family:Carlito,Lato,sans-serif;font-size:14.6667px;line-height:1.2;color:#000;}
.xl-scroll{overflow:auto;max-width:100%;}
.xl-grid{position:relative;display:inline-block;}
.xl-sheet{border-collapse:collapse;table-layout:fixed;background:#fff;border-spacing:0;}
.xl-sheet td{padding:0 3px;vertical-align:bottom;white-space:pre;overflow:hidden;}
.xl-lines td{border:1px solid #dcdcdc;}
td.xl-num{text-align:right;}
td.xl-bool,td.xl-err{text-align:center;}
th.xl-ch,th.xl-rh,th.xl-corner{background:var(--bg-card,#ececec);color:var(--fg,#1f1f1f);\
border:1px solid var(--border,#c0c0c0);font-weight:500;font-size:11px;letter-spacing:.02em;\
padding:0 4px;text-align:center;vertical-align:middle;position:relative;}
th.xl-rh{text-align:right;}
th.xl-corner{width:46px;}
/* The gutter's colour is painted once behind the whole strip, on the column and
   the header row group, not only on the cells. Cell boxes are scaled by the
   reader's zoom, so adjacent edges land on fractional device pixels and the seam
   between two rows shows whatever is underneath — which for the row gutter is the
   sheet's paper white, i.e. a flickering hairline that moves as you zoom. Table
   paint order is table, columns, row groups, rows, cells, so a column background
   sits directly under the cells with no seams of its own. */
col.xl-cg,.xl-sheet thead{background:var(--bg-card,#ececec);}
.xl-drawings{position:absolute;left:46px;top:21px;width:0;height:0;pointer-events:none;}
.xl-dw{position:absolute;}
.xl-ph{position:absolute;display:flex;align-items:center;justify-content:center;\
border:1px dashed #b0b0b0;background:#f7f7f7;color:#6b6b6b;font-size:11px;text-align:center;}
.xl-empty{color:var(--fg-mute,#6b6b6b);padding:8px 2px;}
.office-note{color:var(--fg-mute,#6b6b6b);font-size:11px;padding:6px 2px;}
.xl-cg{width:46px;}
td.xl-fz{background:#fff;}
.xl-frozen{border-collapse:separate;}
.xl-frozen .xl-fzr,.xl-frozen .xl-fzc{position:sticky;z-index:2;}
.xl-frozen .xl-fzr.xl-fzc{z-index:3;}
.xl-frozen th.xl-fzr,.xl-frozen th.xl-fzc{z-index:4;}
.fzg{left:0;}
.fzh{top:0;}
";

pub struct SheetOut {
    pub html: String,
    pub truncated: bool,
}

/// Per-column geometry, indexed by 0-based column.
struct ColInfo {
    width: f32,
    hidden: bool,
    /// `<col style>`: the xf applied to cells of this column that carry none.
    style: Option<u32>,
}

impl ColInfo {
    fn new(width: f32) -> ColInfo {
        ColInfo {
            width,
            hidden: false,
            style: None,
        }
    }
}

#[derive(Clone, Copy)]
struct Merge {
    r0: u32,
    r1: u32,
    c0: usize,
    c1: usize,
    /// Where the span is actually emitted: the first *visible* row and column of
    /// the range. Usually `(r0, c0)`, but a merge can be anchored in a hidden row
    /// or column, and that cell is never emitted — see `resolve_anchors`.
    ar: u32,
    ac: usize,
}

/// Deduplicated numeric class values (`width`, `height`): most columns of a sheet
/// share one width, so one rule per distinct value keeps the stylesheet small.
#[derive(Default)]
struct ClassMap {
    values: Vec<i32>,
}

impl ClassMap {
    /// Class ordinal for `px`, rounded to whole pixels (sub-pixel table tracks
    /// are not reproducible anyway).
    fn id(&mut self, px: f32) -> usize {
        let key = px.round() as i32;
        match self.values.iter().position(|v| *v == key) {
            Some(i) => i,
            None => {
                self.values.push(key);
                self.values.len() - 1
            }
        }
    }

    fn rules(&self, prefix: &str, prop: &str) -> String {
        let mut out = String::new();
        for (i, v) in self.values.iter().enumerate() {
            out.push('.');
            out.push_str(prefix);
            out.push_str(&i.to_string());
            out.push('{');
            out.push_str(prop);
            out.push(':');
            out.push_str(&v.to_string());
            out.push_str("px;}\n");
        }
        out
    }
}

/// Pixel offsets of the emitted grid, for placing the drawing overlay.
struct Grid {
    /// Left edge of each column, plus one past the last. Hidden columns are zero
    /// width here, exactly as they are in the table.
    col_left: Vec<f32>,
    /// Top edge of each row *number* (index 0 unused), plus one past the last.
    row_top: Vec<f32>,
}

impl Grid {
    fn x(&self, col: usize) -> f32 {
        *self
            .col_left
            .get(col)
            .unwrap_or_else(|| self.col_left.last().unwrap_or(&0.0))
    }

    fn y(&self, row_num: u32) -> f32 {
        *self
            .row_top
            .get(row_num as usize)
            .unwrap_or_else(|| self.row_top.last().unwrap_or(&0.0))
    }
}

// ── entry point ──────────────────────────────────────────────────────────────

pub fn render(ctx: &mut Ctx, sh: &SheetRef) -> Result<SheetOut, String> {
    let Some(part) = sh.part.clone() else {
        return Err("this sheet is not present in the workbook package".to_string());
    };
    let sheet_xml = pkg::read_entry(ctx.zip, &part, ctx.budget)?
        .ok_or_else(|| format!("xlsx: missing sheet part {part}"))?;
    let doc = xml::parse(&sheet_xml)?;
    let root = doc.root_element();
    if root.tag_name().name() != "worksheet" {
        note(
            ctx.notes,
            "This sheet has no cell grid (it is a chart or macro sheet).",
        );
        return Ok(SheetOut {
            html: wrap_style("", empty_body("This sheet has no cell grid.")),
            truncated: false,
        });
    }

    // Shared references are copied out of `ctx` so the emission loop can keep a
    // mutable borrow of the marker/notes fields at the same time.
    let styles = ctx.styles;
    let sst = ctx.sst;
    let terms = ctx.terms;
    let date1904 = ctx.date1904;

    // ── sheet-level settings ────────────────────────────────────────────────
    let mut show_lines = true;
    let mut frozen_rows = 0usize;
    let mut frozen_cols = 0usize;
    if let Some(view) = child(root, "sheetViews").and_then(|v| child(v, "sheetView")) {
        if let Some(v) = bool_attr(view, "showGridLines") {
            show_lines = v;
        }
        if let Some(pane) = child(view, "pane") {
            let state = xml::attr_local(pane, "state").unwrap_or("split");
            if state == "frozen" || state == "frozenSplit" {
                frozen_cols = u32_attr(pane, "xSplit").unwrap_or(0) as usize;
                frozen_rows = u32_attr(pane, "ySplit").unwrap_or(0) as usize;
            }
        }
    }

    let fmt_pr = child(root, "sheetFormatPr");
    let def_col_px = fmt_pr
        .and_then(|n| f32_attr(n, "defaultColWidth"))
        .map(chars_to_px)
        .unwrap_or_else(|| chars_to_px(DEFAULT_COL_CHARS));
    let def_row_px = fmt_pr
        .and_then(|n| f32_attr(n, "defaultRowHeight"))
        .map(|pt| pt_to_px(pt).clamp(1.0, MAX_ROW_PX).round())
        .unwrap_or_else(|| pt_to_px(DEFAULT_ROW_PT).round());

    // ── columns ─────────────────────────────────────────────────────────────
    let mut cols: Vec<ColInfo> = (0..super::MAX_COLS).map(|_| ColInfo::new(def_col_px)).collect();
    if let Some(list) = child(root, "cols") {
        for c in elems(list)
            .filter(|n| n.tag_name().name() == "col")
            .take(MAX_COL_DEFS)
        {
            // `min`/`max` are 1-based and inclusive; a single definition routinely
            // covers every column in the sheet.
            let min = u32_attr(c, "min").unwrap_or(1).max(1) as usize;
            let max = u32_attr(c, "max").unwrap_or(min as u32).max(1) as usize;
            if min > super::MAX_COLS {
                continue;
            }
            // `customWidth`/`customHeight` only record whether the author set the
            // measurement or Excel computed it; both are stored values and both are
            // honoured, so only presence matters here.
            let width = f32_attr(c, "width").map(chars_to_px);
            let hidden = bool_attr(c, "hidden").unwrap_or(false);
            let style = u32_attr(c, "style");
            for i in min..=max.min(super::MAX_COLS) {
                let info = &mut cols[i - 1];
                if let Some(w) = width {
                    info.width = w;
                }
                info.hidden = hidden;
                info.style = style;
            }
        }
    }
    // Collapse every track to the whole pixel the table will actually lay out.
    // The `<col>` rule is rounded either way, so leaving the geometry fractional
    // makes the drawing overlay drift off the grid — the default width alone is
    // 64.01px, which puts a picture anchored at column 200 two pixels adrift.
    for info in cols.iter_mut() {
        info.width = info.width.clamp(MIN_COL_PX, MAX_COL_PX).round();
    }

    // ── rows: bounds and lookup ─────────────────────────────────────────────
    let mut row_nodes: Vec<Option<roxmltree::Node>> = vec![None; super::MAX_ROWS as usize + 1];
    let mut last_row: u32 = 0;
    let mut ncols: usize = 0;
    let mut rows_clipped = false;
    let mut cols_clipped = false;
    if let Some(data) = child(root, "sheetData") {
        let mut implied_row: u32 = 0;
        for rn in elems(data).filter(|n| n.tag_name().name() == "row") {
            let r = u32_attr(rn, "r").unwrap_or(implied_row + 1);
            if r == 0 || r > super::MAX_ROW_NUMBER {
                continue;
            }
            implied_row = r;
            // The extent is taken from cells that *show* something, not from cells
            // that merely exist. Excel writes the used range out in full, so a
            // one-page form routinely carries a thousand rows of `<c s="4"/>` with
            // no value and an xf that paints nothing — 30k cells of nothing, which
            // dominates both the payload and every relayout the preview does.
            let mut inked = false;
            let mut implied_col: usize = 0;
            for cn in elems(rn).filter(|n| n.tag_name().name() == "c") {
                let Some(c) = cell_col(cn, implied_col) else {
                    continue;
                };
                implied_col = c + 1;
                if !cell_inked(cn, styles) {
                    continue;
                }
                inked = true;
                if c >= super::MAX_COLS {
                    cols_clipped = true;
                    continue;
                }
                ncols = ncols.max(c + 1);
            }
            if !inked {
                continue;
            }
            if r > super::MAX_ROWS {
                rows_clipped = true;
                continue;
            }
            last_row = last_row.max(r);
        }
    }

    // A merge anchored inside the inked extent drags the rest of its range in with
    // it: half a merged title is worse than a few blank tracks. Runs before the
    // node lookup below, which needs the final extent.
    //
    // Bounded, because whole-column and whole-row merges are common (`A1:B1048576`
    // is how "merge across a column" is stored) and one of those would undo the
    // whole trim. A merge that reaches further than this is not a title block; it
    // gets clamped to the extent by `parse_merge` instead, as before.
    const MERGE_EXTEND_ROWS: u32 = 64;
    const MERGE_EXTEND_COLS: usize = 32;
    if last_row > 0 && ncols > 0 {
        if let Some(list) = child(root, "mergeCells") {
            for m in elems(list)
                .filter(|n| n.tag_name().name() == "mergeCell")
                .take(MAX_MERGES)
            {
                let Some((r0, c0, r1, c1)) = merge_bounds(xml::attr_local(m, "ref").unwrap_or(""))
                else {
                    continue;
                };
                if r0 <= last_row && r1 > last_row && r1 - last_row <= MERGE_EXTEND_ROWS {
                    rows_clipped |= r1 > super::MAX_ROWS;
                    last_row = r1.min(super::MAX_ROWS);
                }
                if c0 < ncols && c1 >= ncols && c1 + 1 - ncols <= MERGE_EXTEND_COLS {
                    cols_clipped |= c1 >= super::MAX_COLS;
                    ncols = (c1 + 1).min(super::MAX_COLS);
                }
            }
        }
    }

    // Node lookup, once the extent is settled. A blank row inside the extent still
    // needs its node — its height and `customFormat` apply either way.
    if let Some(data) = child(root, "sheetData") {
        let mut implied_row: u32 = 0;
        for rn in elems(data).filter(|n| n.tag_name().name() == "row") {
            let r = u32_attr(rn, "r").unwrap_or(implied_row + 1);
            if r == 0 || r > super::MAX_ROW_NUMBER {
                continue;
            }
            implied_row = r;
            if r <= last_row {
                row_nodes[r as usize] = Some(rn);
            }
        }
    }

    if last_row == 0 || ncols == 0 {
        return Ok(SheetOut {
            html: wrap_style("", empty_body("This sheet is empty.")),
            truncated: false,
        });
    }
    let nrows = last_row.min(super::MAX_ROWS);

    // ── merges ──────────────────────────────────────────────────────────────
    let mut merges: Vec<Merge> = Vec::new();
    if let Some(list) = child(root, "mergeCells") {
        for m in elems(list)
            .filter(|n| n.tag_name().name() == "mergeCell")
            .take(MAX_MERGES)
        {
            if let Some(m) = parse_merge(xml::attr_local(m, "ref").unwrap_or(""), nrows, ncols) {
                merges.push(m);
            }
        }
    }

    // ── geometry ────────────────────────────────────────────────────────────
    let mut widths = ClassMap::default();
    let mut heights = ClassMap::default();
    let vis_cols: Vec<usize> = (0..ncols).filter(|c| !cols[*c].hidden).collect();
    let mut col_left = vec![0.0f32; ncols + 1];
    {
        let mut x = 0.0f32;
        for c in 0..ncols {
            col_left[c] = x;
            if !cols[c].hidden {
                x += cols[c].width;
            }
        }
        col_left[ncols] = x;
    }
    // Row heights, and the class each row's height maps to.
    let mut row_px = vec![def_row_px; nrows as usize + 2];
    let mut row_hidden = vec![false; nrows as usize + 2];
    for r in 1..=nrows {
        if let Some(rn) = row_nodes[r as usize] {
            if let Some(ht) = f32_attr(rn, "ht") {
                row_px[r as usize] = pt_to_px(ht).clamp(0.0, MAX_ROW_PX).round();
            }
            row_hidden[r as usize] = bool_attr(rn, "hidden").unwrap_or(false);
        }
    }
    let mut row_top = vec![0.0f32; nrows as usize + 2];
    {
        let mut y = 0.0f32;
        for r in 1..=nrows {
            row_top[r as usize] = y;
            if !row_hidden[r as usize] {
                y += row_px[r as usize];
            }
        }
        row_top[nrows as usize + 1] = y;
    }
    let grid = Grid { col_left, row_top };
    resolve_anchors(&mut merges, &row_hidden, &cols);
    // Whether any merge's span moved off its stored top-left cell. Almost never
    // true, and the value-borrowing pass below is skipped entirely when it is not.
    let displaced = merges.iter().any(|m| m.ar != m.r0 || m.ac != m.c0);

    // Frozen panes are the only reason to switch the table to separate borders
    // (`position:sticky` does nothing under `border-collapse:collapse`), and that
    // changes how adjacent borders paint — so it is opt-in per document.
    let frozen_cols = frozen_cols.min(MAX_FROZEN).min(ncols);
    let frozen_rows = frozen_rows.min(MAX_FROZEN).min(nrows as usize);
    let frozen = frozen_cols > 0 || frozen_rows > 0;
    let mut frozen_css = String::new();
    if frozen {
        // Sticky offsets accumulate behind the gutter/header chrome. They ride on
        // the *cells*, never on the `<tr>`: sticky positioning on a table row is
        // not reliably implemented, while on a cell it is.
        let mut x = ROW_HDR_PX;
        for (vi, &c) in vis_cols.iter().enumerate() {
            if c >= frozen_cols {
                break;
            }
            frozen_css.push_str(&format!(".fzc{vi}{{left:{}px;}}\n", round(x)));
            x += cols[c].width;
        }
        let mut y = COL_HDR_PX;
        for r in 1..=nrows {
            if r as usize > frozen_rows {
                break;
            }
            if row_hidden[r as usize] {
                continue;
            }
            frozen_css.push_str(&format!(".fzr{r}{{top:{}px;}}\n", round(y)));
            y += row_px[r as usize];
        }
    }

    // ── emit ────────────────────────────────────────────────────────────────
    let mut w = Writer::new(ctx.html_cap);
    let mut used: BTreeSet<u32> = BTreeSet::new();
    let mut num_colors: Vec<&'static str> = Vec::new();

    w.open("div", &attr("class", "xl-doc"));
    w.open("div", &attr("class", "xl-scroll"));
    w.open("div", &attr("class", "xl-grid"));
    let table_class = match (show_lines, frozen) {
        (true, true) => "xl-sheet xl-lines xl-frozen",
        (true, false) => "xl-sheet xl-lines",
        (false, true) => "xl-sheet xl-frozen",
        (false, false) => "xl-sheet",
    };
    w.open("table", &attr("class", table_class));

    w.open("colgroup", "");
    w.void("col", &attr("class", "xl-cg"));
    for &c in &vis_cols {
        let id = widths.id(cols[c].width);
        w.void("col", &attr("class", &format!("xw{id}")));
    }
    w.close();

    w.open("thead", "");
    w.open("tr", "");
    let corner = if frozen {
        "xl-corner xl-fzr fzh xl-fzc fzg"
    } else {
        "xl-corner"
    };
    w.open("th", &attr("class", corner));
    w.close();
    for (vi, &c) in vis_cols.iter().enumerate() {
        let mut cls = String::from("xl-ch");
        if frozen {
            push_class(&mut cls, "xl-fzr fzh");
            if c < frozen_cols {
                push_class(&mut cls, &format!("xl-fzc fzc{vi}"));
            }
        }
        w.open("th", &attr("class", &cls));
        w.text(&col_letter(c));
        w.close();
    }
    w.close();
    w.close();

    w.open("tbody", "");
    // Reused per row so a wide sheet does not reallocate these every row.
    let mut covered = vec![false; ncols];
    let mut spans: Vec<Option<(u32, usize)>> = vec![None; ncols];
    let mut cells: Vec<Option<roxmltree::Node>> = vec![None; ncols];

    'rows: for r in 1..=nrows {
        if row_hidden[r as usize] {
            continue;
        }
        // Merge bookkeeping for this row: which columns a span already covers, and
        // where a span starts. Rebuilt per row so nothing has to materialize a
        // covered-cell set for a merge that runs the height of the sheet.
        covered.iter_mut().for_each(|v| *v = false);
        spans.iter_mut().for_each(|v| *v = None);
        for m in &merges {
            if r < m.r0 || r > m.r1 {
                continue;
            }
            for c in m.c0..=m.c1.min(ncols - 1) {
                if r == m.ar && c == m.ac {
                    // Spans count *emitted* tracks only: hidden rows/columns are
                    // not in the table, so counting them would push the rest of the
                    // row sideways.
                    let rs = (m.r0..=m.r1)
                        .filter(|rr| !row_hidden[*rr as usize])
                        .count()
                        .max(1);
                    let cs = (m.c0..=m.c1.min(ncols - 1))
                        .filter(|cc| !cols[*cc].hidden)
                        .count()
                        .max(1);
                    spans[c] = Some((rs as u32, cs));
                } else {
                    covered[c] = true;
                }
            }
        }

        let row_node = row_nodes[r as usize];
        cells.iter_mut().for_each(|v| *v = None);
        let mut row_style: Option<u32> = None;
        if let Some(rn) = row_node {
            if bool_attr(rn, "customFormat").unwrap_or(false) {
                row_style = u32_attr(rn, "s");
            }
            let mut implied_col = 0usize;
            for cn in elems(rn).filter(|n| n.tag_name().name() == "c") {
                let Some(c) = cell_col(cn, implied_col) else {
                    continue;
                };
                implied_col = c + 1;
                if c < ncols {
                    cells[c] = Some(cn);
                }
            }
        }

        // A merged range stores its value in the top-left cell only. When that cell
        // is in a hidden row or column the span is emitted somewhere else, so the
        // visible anchor borrows the value — otherwise a merged title above a hidden
        // row, or spanning a hidden helper column, renders blank.
        if displaced {
            for m in &merges {
                if r != m.ar || (m.ar == m.r0 && m.ac == m.c0) || cells[m.ac].is_some() {
                    continue;
                }
                let Some(src) = row_nodes[m.r0 as usize] else {
                    continue;
                };
                let mut implied = 0usize;
                for cn in elems(src).filter(|n| n.tag_name().name() == "c") {
                    let Some(c) = cell_col(cn, implied) else {
                        continue;
                    };
                    implied = c + 1;
                    if c == m.c0 {
                        cells[m.ac] = Some(cn);
                        break;
                    }
                }
            }
        }

        let hid = heights.id(row_px[r as usize]);
        let row_frozen = frozen && (r as usize) <= frozen_rows;
        w.open("tr", &attr("class", &format!("xh{hid}")));
        let mut rh_cls = String::from("xl-rh");
        if frozen {
            push_class(&mut rh_cls, "xl-fzc fzg");
            if row_frozen {
                push_class(&mut rh_cls, &format!("xl-fzr fzr{r}"));
            }
        }
        w.open("th", &attr("class", &rh_cls));
        w.text(&r.to_string());
        w.close();

        for (vi, &c) in vis_cols.iter().enumerate() {
            if covered[c] {
                continue;
            }
            let cell = cells[c];
            let style_id = cell
                .and_then(|n| u32_attr(n, "s"))
                .or(row_style)
                .or(cols[c].style)
                .unwrap_or(0);
            let cs = styles.get(style_id);

            let (text, kind) = match cell {
                Some(n) => cell_text(n, &cs.fmt, sst, date1904, ctx.notes),
                None => (String::new(), Kind::Blank),
            };
            let color = cell
                .filter(|_| kind == Kind::Num)
                .and_then(|n| numeric_value(n))
                .and_then(|v| cs.fmt.color(v))
                .filter(|c| is_hex_color(c));

            let mut cls = String::new();
            if styles.has_css(style_id) {
                used.insert(style_id);
                cls.push_str("xf");
                cls.push_str(&style_id.to_string());
            }
            if let Some(k) = kind.class() {
                push_class(&mut cls, k);
            }
            if let Some(col) = color {
                let idx = match num_colors.iter().position(|c| *c == col) {
                    Some(i) => i,
                    None => {
                        num_colors.push(col);
                        num_colors.len() - 1
                    }
                };
                push_class(&mut cls, &format!("xnc{idx}"));
            }
            if frozen && (c < frozen_cols || row_frozen) {
                // Frozen cells need an opaque backdrop so scrolled content does not
                // show through. `td.xl-fz` ties with `td.xfN` on specificity and is
                // emitted first, so a document fill still wins.
                push_class(&mut cls, "xl-fz");
                if c < frozen_cols {
                    push_class(&mut cls, &format!("xl-fzc fzc{vi}"));
                }
                if row_frozen {
                    push_class(&mut cls, &format!("xl-fzr fzr{r}"));
                }
            }

            let (rowspan, colspan) = match spans[c] {
                Some((rs, cols_n)) => (
                    if rs > 1 {
                        attr("rowspan", &rs.to_string())
                    } else {
                        String::new()
                    },
                    if cols_n > 1 {
                        attr("colspan", &cols_n.to_string())
                    } else {
                        String::new()
                    },
                ),
                None => (String::new(), String::new()),
            };
            let class_attr = if cls.is_empty() {
                String::new()
            } else {
                attr("class", &cls)
            };
            w.open("td", &attrs(&[&class_attr, &rowspan, &colspan]));
            if !text.is_empty() {
                let inner = styles.has_inner(style_id);
                if inner {
                    w.open("span", &attr("class", "xr"));
                }
                // `mark` escapes as it wraps matches, so its output is the only
                // document-derived string that may go in raw.
                let marked = ctx.marker.mark(&text, terms);
                w.raw(&marked);
                if inner {
                    w.close();
                }
            }
            w.close();
            if w.is_full() {
                break 'rows;
            }
        }
        w.close();
    }
    w.close(); // tbody
    w.close(); // table

    // ── drawings ────────────────────────────────────────────────────────────
    if let Some(rid) = child(root, "drawing").and_then(|n| xml::attr_local(n, "id")) {
        let rid = rid.to_string();
        if !w.is_full() {
            if let Err(e) = drawings(ctx, &mut w, &part, &rid, &grid) {
                if e == pkg::BUDGET_EXCEEDED {
                    note(
                        ctx.notes,
                        "Some embedded images were skipped: the document exceeds the preview size budget.",
                    );
                } else {
                    note(ctx.notes, "Some embedded images could not be read.");
                }
            }
        }
    }

    w.close(); // xl-grid
    w.close(); // xl-scroll
    w.close(); // xl-doc

    if rows_clipped {
        note(
            ctx.notes,
            &format!("Only the first {} rows are shown.", super::MAX_ROWS),
        );
    }
    if cols_clipped {
        note(
            ctx.notes,
            &format!("Only the first {} columns are shown.", super::MAX_COLS),
        );
    }

    let mut css = String::new();
    css.push_str(&widths.rules("xw", "width"));
    css.push_str(&heights.rules("xh", "height"));
    css.push_str(&frozen_css);
    for (i, c) in num_colors.iter().enumerate() {
        css.push_str(&format!("td.xnc{i}{{color:{c};}}\n"));
    }
    // Style rules last: they must beat the base gridline/alignment rules, which
    // they only do on source order.
    css.push_str(&styles.css_block(used));

    let truncated = w.truncated() || rows_clipped || cols_clipped;
    Ok(SheetOut {
        html: wrap_style(&css, w.finish()),
        truncated,
    })
}

// ── cell values ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Num,
    Text,
    Bool,
    Err,
    Blank,
}

impl Kind {
    /// The class carrying Excel's "General" alignment, which depends on the value
    /// and so cannot live in the per-`xf` rule.
    fn class(self) -> Option<&'static str> {
        match self {
            Kind::Num => Some("xl-num"),
            Kind::Bool | Kind::Err => Some("xl-bool"),
            Kind::Text | Kind::Blank => None,
        }
    }
}

/// The cell's displayed string. `t` selects the storage, and for the numeric case
/// the number format is what turns a serial back into a date — `Format::apply_with`
/// performs that conversion internally for a date-kind code, which is why a bare
/// `45678` never reaches the output.
fn cell_text(
    cell: roxmltree::Node<'_, '_>,
    fmt: &Format,
    sst: &[String],
    date1904: bool,
    notes: &mut Vec<String>,
) -> (String, Kind) {
    let t = xml::attr_local(cell, "t").unwrap_or("n");
    match t {
        "s" => {
            let idx = child(cell, "v")
                .and_then(text_of)
                .and_then(|s| s.trim().parse::<usize>().ok());
            match idx.and_then(|i| sst.get(i)) {
                Some(s) => (clip(fmt.apply_text(s)), Kind::Text),
                None => {
                    note(
                        notes,
                        "Some cell text is missing: the workbook's shared string table is incomplete.",
                    );
                    (String::new(), Kind::Blank)
                }
            }
        }
        // An inline string is a full rich-text body, so its runs are joined the
        // same way a shared string's are.
        "inlineStr" => match child(cell, "is") {
            Some(is) => (clip(fmt.apply_text(&rich_text(is))), Kind::Text),
            None => (String::new(), Kind::Blank),
        },
        // A formula's cached string result.
        "str" => match child(cell, "v").and_then(text_of) {
            Some(s) => (clip(fmt.apply_text(s)), Kind::Text),
            None => (String::new(), Kind::Blank),
        },
        "b" => match child(cell, "v").and_then(text_of) {
            Some(v) => (
                if v.trim() == "0" { "FALSE" } else { "TRUE" }.to_string(),
                Kind::Bool,
            ),
            None => (String::new(), Kind::Blank),
        },
        "e" => match child(cell, "v").and_then(text_of) {
            Some(v) => (clip(v.trim().to_string()), Kind::Err),
            None => (String::new(), Kind::Blank),
        },
        // ISO 8601 in the cell itself (`t="d"`, rare outside Office 365). The text
        // is already human-readable, so it is passed through rather than re-parsed
        // into a serial just to format it again.
        "d" => match child(cell, "v").and_then(text_of) {
            Some(v) => (clip(v.trim().to_string()), Kind::Text),
            None => (String::new(), Kind::Blank),
        },
        _ => match child(cell, "v").and_then(text_of) {
            Some(v) => match v.trim().parse::<f64>() {
                Ok(n) if n.is_finite() => (clip(fmt.apply_with(n, date1904)), Kind::Num),
                // A non-numeric `<v>` in a numeric cell is malformed; showing the
                // raw text beats showing nothing.
                _ => (clip(v.trim().to_string()), Kind::Text),
            },
            None => (String::new(), Kind::Blank),
        },
    }
}

fn numeric_value(cell: roxmltree::Node<'_, '_>) -> Option<f64> {
    child(cell, "v")?
        .text()?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

/// Concatenates the `<t>` of a rich-text body, skipping the phonetic subtrees.
/// A run-formatted cell arrives as several `<r><t>` fragments — reading only the
/// first would silently drop most of the text — while `<rPh>` holds furigana that
/// is a *reading aid* for the same characters and would duplicate them.
pub(super) fn rich_text(node: roxmltree::Node<'_, '_>) -> String {
    let mut out = String::new();
    collect_text(node, &mut out);
    out
}

fn collect_text(node: roxmltree::Node<'_, '_>, out: &mut String) {
    for ch in node.children() {
        if !ch.is_element() {
            continue;
        }
        match ch.tag_name().name() {
            "rPh" | "phoneticPr" => continue,
            "t" => {
                for d in ch.descendants().filter(|d| d.is_text()) {
                    if let Some(s) = d.text() {
                        out.push_str(s);
                    }
                }
            }
            _ => collect_text(ch, out),
        }
    }
}

fn clip(mut s: String) -> String {
    if s.chars().count() <= MAX_CELL_CHARS {
        return s;
    }
    let cut = s
        .char_indices()
        .nth(MAX_CELL_CHARS)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s.truncate(cut);
    s.push('…');
    s
}

// ── merges ───────────────────────────────────────────────────────────────────

/// `A1:C3` → a merge clamped to the emitted grid. A range that runs past the
/// emitted bounds (a merge down a whole column) is clipped rather than dropped, so
/// its visible part still spans.
/// A merge's normalized `(r0, c0, r1, c1)`, unclamped — rows 1-based, columns
/// 0-based. Used before the sheet's extent is known, to let a merge extend it.
fn merge_bounds(r: &str) -> Option<(u32, usize, u32, usize)> {
    let (a, b) = r.split_once(':')?;
    let (r0, c0) = parse_ref(a)?;
    let (r1, c1) = parse_ref(b)?;
    Some((r0.min(r1), c0.min(c1), r0.max(r1), c0.max(c1)))
}

fn parse_merge(r: &str, nrows: u32, ncols: usize) -> Option<Merge> {
    let (r0, c0, r1, c1) = merge_bounds(r)?;
    if r0 > nrows || c0 >= ncols {
        return None;
    }
    Some(Merge {
        r0,
        r1: r1.min(nrows),
        c0,
        c1: c1.min(ncols.saturating_sub(1)),
        // Provisional; `resolve_anchors` corrects these once visibility is known.
        ar: r0,
        ac: c0,
    })
}

/// Move each merge's anchor to the first visible row/column of its range, and drop
/// merges with nothing visible at all.
///
/// The row loop emits a span at the anchor and suppresses every other cell of the
/// range. If the stored anchor sits in a hidden row or column that cell is never
/// emitted, so the row is one `<td>` short of its headers and everything after it
/// slides left — for a merge hidden at the top of a sheet, that is the whole grid.
fn resolve_anchors(merges: &mut Vec<Merge>, row_hidden: &[bool], cols: &[ColInfo]) {
    merges.retain_mut(|m| {
        let Some(ar) = (m.r0..=m.r1).find(|r| !row_hidden[*r as usize]) else {
            return false;
        };
        let Some(ac) = (m.c0..=m.c1).find(|c| !cols[*c].hidden) else {
            return false;
        };
        m.ar = ar;
        m.ac = ac;
        true
    });
}

fn parse_ref(s: &str) -> Option<(u32, usize)> {
    let s = s.trim().trim_start_matches('$');
    let (letters, digits) = split_cell_ref(s);
    let col = col_letter_to_index(letters)?;
    let row: u32 = digits.trim_start_matches('$').parse().ok()?;
    (row > 0).then_some((row, col))
}

/// Whether a cell contributes to the sheet's visible extent.
///
/// A cell counts when it carries a value (`<v>`, `<is>`, a formula) or when its
/// xf paints something an empty cell would still show — a fill or a border. It
/// does *not* count for a font, an alignment or a number format: those are
/// invisible without text, and Excel stamps them across the whole used range.
fn cell_inked(cell: roxmltree::Node<'_, '_>, styles: &super::styles::Styles) -> bool {
    for n in elems(cell) {
        match n.tag_name().name() {
            "v" | "f" => return true,
            // An `<is>` holding only empty runs is still an empty cell.
            "is" => {
                if n.descendants().any(|d| {
                    d.text()
                        .map(|t| !t.trim().is_empty())
                        .unwrap_or(false)
                }) {
                    return true;
                }
            }
            _ => {}
        }
    }
    u32_attr(cell, "s").is_some_and(|s| styles.paints(s))
}

/// A cell's 0-based column from its `r` attribute, falling back to the position
/// after the previous cell (the `r` attribute is optional).
fn cell_col(cell: roxmltree::Node<'_, '_>, implied: usize) -> Option<usize> {
    match xml::attr_local(cell, "r") {
        Some(r) => col_letter_to_index(split_cell_ref(r).0),
        None => Some(implied),
    }
}

// ── drawings ─────────────────────────────────────────────────────────────────

/// Images and charts anchored over the grid.
///
/// Positions are computed from the *emitted* column/row offsets, so a sheet with
/// hidden rows or columns places its pictures where the visible grid puts those
/// cells rather than where Excel would — the alternative is a picture floating
/// over unrelated data.
fn drawings(
    ctx: &mut Ctx,
    w: &mut Writer,
    sheet_part: &str,
    rid: &str,
    grid: &Grid,
) -> Result<(), String> {
    let rels = match pkg::read_entry(ctx.zip, &opc::rels_path_for(sheet_part), ctx.budget)? {
        Some(x) => opc::parse_rels(&x)?,
        None => return Ok(()),
    };
    let Some(rel) = rels.get(rid) else {
        return Ok(());
    };
    if rel.external {
        return Ok(());
    }
    let Some(dpart) = opc::resolve_target(sheet_part, &rel.target) else {
        return Ok(());
    };
    let Some(dxml) = pkg::read_entry(ctx.zip, &dpart, ctx.budget)? else {
        return Ok(());
    };
    let ddoc = xml::parse(&dxml)?;
    let drels = match pkg::read_entry(ctx.zip, &opc::rels_path_for(&dpart), ctx.budget)? {
        Some(x) => opc::parse_rels(&x).unwrap_or_default(),
        None => Default::default(),
    };

    let mut opened = false;
    let mut unsupported = false;
    for anchor in elems(ddoc.root_element()).take(MAX_DRAWINGS) {
        if w.is_full() {
            break;
        }
        let Some(xf) = anchor_box(anchor, grid) else {
            continue;
        };
        if xf.cx < 1.0 || xf.cy < 1.0 {
            continue;
        }
        // `mc:AlternateContent` usually pairs a metafile with a raster fallback;
        // the fallback is the branch a consumer of no extension namespaces must
        // take, and it is the one that can actually be displayed.
        let scope = elems(anchor)
            .find(|n| n.tag_name().name() == "AlternateContent")
            .and_then(media::prefer_raster_branch)
            .unwrap_or(anchor);

        let embed = descendant(scope, "blip").and_then(|b| xml::attr_local(b, "embed"));
        let label = chart_label(scope);
        if embed.is_none() && label.is_none() {
            unsupported = true;
            continue;
        }
        if !opened {
            w.open("div", &attr("class", "xl-drawings"));
            opened = true;
        }
        let style = xf.css();
        if let Some(id) = embed {
            let target = drels
                .get(id)
                .filter(|r| !r.external)
                .and_then(|r| opc::resolve_target(&dpart, &r.target));
            let media = match target {
                Some(part) => {
                    ctx.media
                        .get(ctx.zip, ctx.budget, ctx.mb, &part, xf.cx.round() as u32)
                }
                None => Media::Placeholder("image unavailable"),
            };
            match media {
                Media::DataUri(uri) => {
                    let alt = descendant(scope, "cNvPr")
                        .and_then(|p| {
                            xml::attr_local(p, "descr").or_else(|| xml::attr_local(p, "name"))
                        })
                        .unwrap_or("");
                    w.void(
                        "img",
                        &attrs(&[
                            &attr("class", "xl-dw"),
                            &attr("style", &style),
                            &attr("src", &uri),
                            &attr("alt", alt),
                        ]),
                    );
                }
                Media::Placeholder(reason) => placeholder(w, &style, reason),
            }
        } else if let Some(l) = label {
            placeholder(w, &style, l);
        }
    }
    if opened {
        w.close();
    }
    if unsupported {
        note(
            ctx.notes,
            "Some drawing shapes are not shown in the preview.",
        );
    }
    Ok(())
}

fn placeholder(w: &mut Writer, style: &str, label: &str) {
    w.open(
        "div",
        &attrs(&[&attr("class", "xl-ph"), &attr("style", style)]),
    );
    w.text(label);
    w.close();
}

/// A label for a graphic frame the preview cannot rasterize. Charts are the
/// common case: they are stored as data plus a layout, never as an image, so a
/// labelled box at the right geometry is the honest answer.
fn chart_label(scope: roxmltree::Node<'_, '_>) -> Option<&'static str> {
    let uri = descendant(scope, "graphicData").and_then(|g| xml::attr_local(g, "uri"))?;
    Some(if uri.contains("/chart") {
        "Chart"
    } else if uri.contains("/diagram") {
        "Diagram"
    } else if uri.contains("/table") {
        "Table"
    } else {
        "Embedded object"
    })
}

/// Pixel box of a drawing anchor. All three anchor kinds are handled: two-cell
/// (a from/to cell pair), one-cell (a from cell plus an extent) and absolute (a
/// slide-style position plus an extent).
fn anchor_box(anchor: roxmltree::Node<'_, '_>, grid: &Grid) -> Option<Xf> {
    let kind = anchor.tag_name().name();
    let ext = child(anchor, "ext");
    let (x, y, cx, cy) = match kind {
        "twoCellAnchor" => {
            let (x0, y0) = cell_point(child(anchor, "from")?, grid)?;
            let (x1, y1) = cell_point(child(anchor, "to")?, grid)?;
            (x0, y0, x1 - x0, y1 - y0)
        }
        "oneCellAnchor" => {
            let (x0, y0) = cell_point(child(anchor, "from")?, grid)?;
            let e = ext?;
            (
                x0,
                y0,
                emu_to_px(i64_attr(e, "cx")?),
                emu_to_px(i64_attr(e, "cy")?),
            )
        }
        "absoluteAnchor" => {
            let pos = child(anchor, "pos")?;
            let e = ext?;
            (
                emu_to_px(i64_attr(pos, "x")?),
                emu_to_px(i64_attr(pos, "y")?),
                emu_to_px(i64_attr(e, "cx")?),
                emu_to_px(i64_attr(e, "cy")?),
            )
        }
        _ => return None,
    };
    if ![x, y, cx, cy].iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(Xf {
        x: x as f64,
        y: y as f64,
        cx: cx as f64,
        cy: cy as f64,
        rot: 0.0,
        flip_h: false,
        flip_v: false,
    })
}

/// `<xdr:from>` / `<xdr:to>`: a 0-based cell index plus an EMU offset inside it.
fn cell_point(node: roxmltree::Node<'_, '_>, grid: &Grid) -> Option<(f32, f32)> {
    let col = child(node, "col").and_then(text_of)?.trim().parse::<u32>().ok()? as usize;
    let row = child(node, "row").and_then(text_of)?.trim().parse::<u32>().ok()?;
    let col_off = child(node, "colOff")
        .and_then(text_of)
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    let row_off = child(node, "rowOff")
        .and_then(text_of)
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    Some((
        grid.x(col) + emu_to_px(col_off),
        grid.y(row.saturating_add(1)) + emu_to_px(row_off),
    ))
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn wrap_style(extra_css: &str, body: String) -> String {
    let mut out = String::with_capacity(BASE_CSS.len() + extra_css.len() + body.len() + 32);
    out.push_str("<style>");
    // Defensive: nothing generated here contains `<`, and a `</style>` smuggled
    // into a font name or colour would end the element and turn the rest of the
    // stylesheet into document content.
    out.push_str(&BASE_CSS.replace('<', ""));
    out.push_str(&extra_css.replace('<', ""));
    out.push_str("</style>");
    out.push_str(&body);
    out
}

/// Body for a sheet that could not be rendered at all. Unlike `empty_body` this
/// is self-contained — it carries the base stylesheet, because it stands in for
/// a whole section's html rather than being nested inside a normal render.
pub(super) fn error_body(msg: &str) -> String {
    wrap_style("", empty_body(msg))
}

fn empty_body(msg: &str) -> String {
    let mut w = Writer::new(1024);
    w.open("div", &attr("class", "xl-doc"));
    w.open("div", &attr("class", "xl-empty"));
    w.text(msg);
    w.close();
    w.close();
    w.finish()
}

fn push_class(cls: &mut String, add: &str) {
    if !cls.is_empty() {
        cls.push(' ');
    }
    cls.push_str(add);
}

/// Stored column width (in default-font digits) → px.
fn chars_to_px(chars: f32) -> f32 {
    if !chars.is_finite() {
        return chars_to_px(DEFAULT_COL_CHARS);
    }
    (chars * MDW_PX + COL_PAD_PX).clamp(0.0, MAX_COL_PX)
}

fn round(v: f32) -> i32 {
    if v.is_finite() {
        v.round() as i32
    } else {
        0
    }
}

/// Guards the one CSS value that comes from a parsed format code rather than
/// from a fixed table.
fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn child<'a>(node: roxmltree::Node<'a, 'a>, local: &str) -> Option<roxmltree::Node<'a, 'a>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == local)
}

fn descendant<'a>(node: roxmltree::Node<'a, 'a>, local: &str) -> Option<roxmltree::Node<'a, 'a>> {
    node.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == local)
}

fn elems<'a>(node: roxmltree::Node<'a, 'a>) -> impl Iterator<Item = roxmltree::Node<'a, 'a>> + 'a {
    node.children().filter(|n| n.is_element())
}

fn text_of<'a>(node: roxmltree::Node<'a, 'a>) -> Option<&'a str> {
    node.descendants().find(|n| n.is_text()).and_then(|n| n.text())
}

fn i64_attr(node: roxmltree::Node<'_, '_>, local: &str) -> Option<i64> {
    xml::attr_local(node, local)?.trim().parse().ok()
}
