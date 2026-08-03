//! One worksheet → an HTML `<table>` with a row/column gutter, resolved cell
//! formats as classes, merged-cell spans, frozen panes and the drawing overlay.

use super::super::drawingml::geom::Xf;
use super::super::emit::{self, Notes};
use super::super::highlight::{Marker, Terms};
use super::super::html::{attr, attrs, emu_to_px, pt_to_px, Writer};
use super::super::media::{self, Media};
use super::super::model::Align;
use super::super::numfmt::Format;
use super::super::sheetmodel::{resolve_anchors, Cell, CellSource, Merge, Track};
use super::super::xml::{
    self, attr_bool, attr_f32, attr_i64, attr_u32, child, descendant, elems, text_of,
};
use super::super::{opc, pkg};
use super::{col_letter, col_letter_to_index, split_cell_ref, Ctx, SheetRef};
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
/* Cells with text take the I-beam; everything else keeps body's grab cursor,
   because everything else pans. See the `xl-t` note in the cell loop. */
td.xl-t{cursor:text;}
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

// ── phase state ──────────────────────────────────────────────────────────────
//
// `render` is a pipeline: parse the SpreadsheetML into these structs, turn them
// into a `Layout`, then emit. Everything from `Layout` on is format-neutral — it
// speaks only the `sheetmodel` vocabulary — so a second spreadsheet dialect has
// to reach this seam and no further.

/// Sheet-level view flags and default track sizes.
///
/// SpreadsheetML-specific: `<sheetView>`, `<pane>` and `<sheetFormatPr>` are
/// OOXML spellings with no shared shape in ODF, which stores the equivalents on
/// styles and in a separate settings part.
struct Settings {
    show_lines: bool,
    /// The pane split as stored, before it is clamped to the emitted grid.
    frozen_rows: usize,
    frozen_cols: usize,
    def_col_px: f32,
    def_row_px: f32,
}

/// The sheet's inked rectangle — rows `1..=last_row`, columns `0..ncols` — and
/// whether either axis was cut by the emission caps.
struct Extent {
    last_row: u32,
    ncols: usize,
    rows_clipped: bool,
    cols_clipped: bool,
}

/// Frozen pane depth in track counts, already clamped to `MAX_FROZEN` and to the
/// emitted grid. Format-neutral.
#[derive(Clone, Copy)]
struct Frozen {
    rows: usize,
    cols: usize,
}

impl Frozen {
    /// Frozen panes are the only reason to switch the table to separate borders
    /// (`position:sticky` does nothing under `border-collapse:collapse`), and that
    /// changes how adjacent borders paint — so it is opt-in per document.
    fn clamp(settings: &Settings, nrows: u32, ncols: usize) -> Frozen {
        Frozen {
            rows: settings.frozen_rows.min(MAX_FROZEN).min(nrows as usize),
            cols: settings.frozen_cols.min(MAX_FROZEN).min(ncols),
        }
    }

    fn on(self) -> bool {
        self.rows > 0 || self.cols > 0
    }
}

/// The emitted grid's shape and size — everything the emission half needs, with
/// no reference to the XML it was measured from.
///
/// Format-neutral, and the seam a second dialect joins at: fill this in, hand the
/// emitter a [`CellSource`], and the whole emission half is reusable.
struct Layout {
    nrows: u32,
    ncols: usize,
    /// Indexed by 0-based column, trimmed to `ncols`.
    cols: Vec<Track>,
    /// The emitted columns, in order; hidden ones are absent.
    vis_cols: Vec<usize>,
    /// Indexed by 1-based row number, as `row_tracks` builds it.
    rows: Vec<Track>,
    grid: Grid,
}

/// Class tables filled while emitting and turned into rules afterwards: the
/// stylesheet cannot be written before the grid that references it is walked.
#[derive(Default)]
struct Classes {
    widths: ClassMap,
    heights: ClassMap,
    /// Number-format colours, deduplicated in first-seen order.
    num_colors: Vec<String>,
    /// The `xf`s some emitted cell actually referenced.
    used: BTreeSet<u32>,
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
        ctx.notes
            .add("This sheet has no cell grid (it is a chart or macro sheet).");
        return Ok(SheetOut {
            html: error_body("This sheet has no cell grid."),
            truncated: false,
        });
    }

    // Shared references are copied out of `ctx` so the emission loop can keep a
    // mutable borrow of the marker/notes fields at the same time.
    let styles = ctx.styles;
    let sst = ctx.sst;
    let terms = ctx.terms;
    let date1904 = ctx.date1904;

    // ── parse ───────────────────────────────────────────────────────────────
    let settings = sheet_settings(root);
    let cols = parse_cols(root, settings.def_col_px);
    let extent = used_extent(root, styles);
    if extent.last_row == 0 || extent.ncols == 0 {
        return Ok(SheetOut {
            html: error_body("This sheet is empty."),
            truncated: false,
        });
    }
    let nrows = extent.last_row.min(super::MAX_ROWS);
    let ncols = extent.ncols;
    let row_nodes = row_lookup(root, nrows);
    let mut merges = parse_merges(root, nrows, ncols);

    // ── geometry ────────────────────────────────────────────────────────────
    let rows = row_tracks(&row_nodes, nrows, settings.def_row_px);
    let layout = build_geometry(cols, rows, nrows, ncols);
    resolve_anchors(&mut merges, &layout.rows, &layout.cols);
    let frozen = Frozen::clamp(&settings, nrows, ncols);
    let frozen_css = frozen_pane_css(&layout, frozen);

    // ── emit ────────────────────────────────────────────────────────────────
    let mut w = Writer::new(ctx.html_cap);
    let mut classes = Classes::default();
    emit_head(&mut w, &layout, frozen, settings.show_lines, &mut classes);
    let mut src = SheetRows {
        row_nodes: &row_nodes,
        rows: &layout.rows,
        cols: &layout.cols,
        merges: &merges,
        styles,
        sst,
        date1904,
        notes: &mut *ctx.notes,
        // Almost never true, and the value-borrowing pass in `row` is skipped
        // entirely when it is not.
        displaced: merges.iter().any(|m| m.ar != m.r0 || m.ac != m.c0),
        r: 0,
        cells: vec![None; ncols],
    };
    emit_rows(
        &mut w,
        &layout,
        &merges,
        &mut src,
        frozen,
        &mut *ctx.marker,
        terms,
        &mut classes,
    );
    w.close(); // table

    // ── drawings ────────────────────────────────────────────────────────────
    if let Some(rid) = child(root, "drawing").and_then(|n| xml::attr_local(n, "id")) {
        let rid = rid.to_string();
        if !w.is_full() {
            if let Err(e) = drawings(ctx, &mut w, &part, &rid, &layout.grid) {
                if e == pkg::BUDGET_EXCEEDED {
                    ctx.notes.add(
                        "Some embedded images were skipped: the document exceeds the preview size budget.",
                    );
                } else {
                    ctx.notes.add("Some embedded images could not be read.");
                }
            }
        }
    }

    w.close(); // xl-grid
    w.close(); // xl-scroll
    w.close(); // xl-doc

    if extent.rows_clipped {
        ctx.notes
            .add(&format!("Only the first {} rows are shown.", super::MAX_ROWS));
    }
    if extent.cols_clipped {
        ctx.notes.add(&format!(
            "Only the first {} columns are shown.",
            super::MAX_COLS
        ));
    }

    let truncated = w.truncated() || extent.rows_clipped || extent.cols_clipped;
    Ok(SheetOut {
        html: emit::wrap_style(BASE_CSS, &collect_css(classes, &frozen_css, styles), w.finish()),
        truncated,
    })
}

// ── parsing ──────────────────────────────────────────────────────────────────

/// Gridlines, the frozen split and the sheet's default track sizes.
///
/// SpreadsheetML-specific.
fn sheet_settings(root: roxmltree::Node<'_, '_>) -> Settings {
    let mut show_lines = true;
    let mut frozen_rows = 0usize;
    let mut frozen_cols = 0usize;
    if let Some(view) = child(root, "sheetViews").and_then(|v| child(v, "sheetView")) {
        if let Some(v) = attr_bool(view, "showGridLines") {
            show_lines = v;
        }
        if let Some(pane) = child(view, "pane") {
            let state = xml::attr_local(pane, "state").unwrap_or("split");
            if state == "frozen" || state == "frozenSplit" {
                frozen_cols = attr_u32(pane, "xSplit").unwrap_or(0) as usize;
                frozen_rows = attr_u32(pane, "ySplit").unwrap_or(0) as usize;
            }
        }
    }

    let fmt_pr = child(root, "sheetFormatPr");
    let def_col_px = fmt_pr
        .and_then(|n| attr_f32(n, "defaultColWidth"))
        .map(chars_to_px)
        .unwrap_or_else(|| chars_to_px(DEFAULT_COL_CHARS));
    let def_row_px = fmt_pr
        .and_then(|n| attr_f32(n, "defaultRowHeight"))
        .map(|pt| pt_to_px(pt).clamp(1.0, MAX_ROW_PX).round())
        .unwrap_or_else(|| pt_to_px(DEFAULT_ROW_PT).round());

    Settings {
        show_lines,
        frozen_rows,
        frozen_cols,
        def_col_px,
        def_row_px,
    }
}

/// `<cols>` → per-column width, visibility and default xf, one entry per
/// possible column. SpreadsheetML-specific.
fn parse_cols(root: roxmltree::Node<'_, '_>, def_col_px: f32) -> Vec<Track> {
    let mut cols: Vec<Track> = (0..super::MAX_COLS).map(|_| Track::new(def_col_px)).collect();
    if let Some(list) = child(root, "cols") {
        for c in elems(list)
            .filter(|n| n.tag_name().name() == "col")
            .take(MAX_COL_DEFS)
        {
            // `min`/`max` are 1-based and inclusive; a single definition routinely
            // covers every column in the sheet.
            let min = attr_u32(c, "min").unwrap_or(1).max(1) as usize;
            let max = attr_u32(c, "max").unwrap_or(min as u32).max(1) as usize;
            if min > super::MAX_COLS {
                continue;
            }
            // `customWidth`/`customHeight` only record whether the author set the
            // measurement or Excel computed it; both are stored values and both are
            // honoured, so only presence matters here.
            let width = attr_f32(c, "width").map(chars_to_px);
            let hidden = attr_bool(c, "hidden").unwrap_or(false);
            let style = attr_u32(c, "style");
            for i in min..=max.min(super::MAX_COLS) {
                let track = &mut cols[i - 1];
                if let Some(w) = width {
                    track.px = w;
                }
                track.hidden = hidden;
                track.style = style;
            }
        }
    }
    // Collapse every track to the whole pixel the table will actually lay out.
    // The `<col>` rule is rounded either way, so leaving the geometry fractional
    // makes the drawing overlay drift off the grid — the default width alone is
    // 64.01px, which puts a picture anchored at column 200 two pixels adrift.
    for track in cols.iter_mut() {
        track.px = track.px.clamp(MIN_COL_PX, MAX_COL_PX).round();
    }
    cols
}

/// The rectangle worth emitting, from `<sheetData>` and then from `<mergeCells>`.
///
/// SpreadsheetML-specific: it walks cell nodes. What it produces — a rectangle
/// plus two clipped flags — is not.
fn used_extent(root: roxmltree::Node<'_, '_>, styles: &super::styles::Styles) -> Extent {
    let mut extent = Extent {
        last_row: 0,
        ncols: 0,
        rows_clipped: false,
        cols_clipped: false,
    };
    if let Some(data) = child(root, "sheetData") {
        let mut implied_row: u32 = 0;
        for rn in elems(data).filter(|n| n.tag_name().name() == "row") {
            let r = attr_u32(rn, "r").unwrap_or(implied_row + 1);
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
                    extent.cols_clipped = true;
                    continue;
                }
                extent.ncols = extent.ncols.max(c + 1);
            }
            if !inked {
                continue;
            }
            if r > super::MAX_ROWS {
                extent.rows_clipped = true;
                continue;
            }
            extent.last_row = extent.last_row.max(r);
        }
    }
    extend_by_merges(root, &mut extent);
    extent
}

/// A merge anchored inside the inked extent drags the rest of its range in with
/// it: half a merged title is worse than a few blank tracks. Runs before
/// `row_lookup`, which needs the final extent.
///
/// Bounded, because whole-column and whole-row merges are common (`A1:B1048576`
/// is how "merge across a column" is stored) and one of those would undo the
/// whole trim. A merge that reaches further than this is not a title block; it
/// gets clamped to the extent by `parse_merge` instead, as before.
fn extend_by_merges(root: roxmltree::Node<'_, '_>, extent: &mut Extent) {
    const MERGE_EXTEND_ROWS: u32 = 64;
    const MERGE_EXTEND_COLS: usize = 32;
    if extent.last_row == 0 || extent.ncols == 0 {
        return;
    }
    let Some(list) = child(root, "mergeCells") else {
        return;
    };
    for m in elems(list)
        .filter(|n| n.tag_name().name() == "mergeCell")
        .take(MAX_MERGES)
    {
        let Some((r0, c0, r1, c1)) = merge_bounds(xml::attr_local(m, "ref").unwrap_or("")) else {
            continue;
        };
        if r0 <= extent.last_row
            && r1 > extent.last_row
            && r1 - extent.last_row <= MERGE_EXTEND_ROWS
        {
            extent.rows_clipped |= r1 > super::MAX_ROWS;
            extent.last_row = r1.min(super::MAX_ROWS);
        }
        if c0 < extent.ncols && c1 >= extent.ncols && c1 + 1 - extent.ncols <= MERGE_EXTEND_COLS {
            extent.cols_clipped |= c1 >= super::MAX_COLS;
            extent.ncols = (c1 + 1).min(super::MAX_COLS);
        }
    }
}

/// Row node by row number, once the extent is settled. A blank row inside the
/// extent still needs its node — its height and `customFormat` apply either way.
///
/// SpreadsheetML-specific, and the only phase that hands XML nodes downstream —
/// to `SheetRows`, which is where they stop.
fn row_lookup<'a>(
    root: roxmltree::Node<'a, 'a>,
    last_row: u32,
) -> Vec<Option<roxmltree::Node<'a, 'a>>> {
    let mut row_nodes: Vec<Option<roxmltree::Node>> = vec![None; super::MAX_ROWS as usize + 1];
    if let Some(data) = child(root, "sheetData") {
        let mut implied_row: u32 = 0;
        for rn in elems(data).filter(|n| n.tag_name().name() == "row") {
            let r = attr_u32(rn, "r").unwrap_or(implied_row + 1);
            if r == 0 || r > super::MAX_ROW_NUMBER {
                continue;
            }
            implied_row = r;
            if r <= last_row {
                row_nodes[r as usize] = Some(rn);
            }
        }
    }
    row_nodes
}

/// Row heights, visibility and default xf, read off the `<row>` nodes.
///
/// Indexed by 1-based row number: index 0 is unused, and there is one slot past
/// the last row so the geometry pass can write the grid's closing edge without a
/// bounds check.
///
/// SpreadsheetML-specific (`ht`, `hidden`, `customFormat`); everything downstream
/// takes the measurements rather than the nodes, which is what makes
/// `build_geometry` reusable.
fn row_tracks(
    row_nodes: &[Option<roxmltree::Node<'_, '_>>],
    nrows: u32,
    def_row_px: f32,
) -> Vec<Track> {
    let mut rows = vec![Track::new(def_row_px); nrows as usize + 2];
    for r in 1..=nrows {
        let Some(rn) = row_nodes[r as usize] else {
            continue;
        };
        let track = &mut rows[r as usize];
        if let Some(ht) = attr_f32(rn, "ht") {
            track.px = pt_to_px(ht).clamp(0.0, MAX_ROW_PX).round();
        }
        track.hidden = attr_bool(rn, "hidden").unwrap_or(false);
        // `customFormat` is what says the row's `s` applies to the row's cells;
        // without it the attribute records only the row's own formatting.
        if attr_bool(rn, "customFormat").unwrap_or(false) {
            track.style = attr_u32(rn, "s");
        }
    }
    rows
}

// ── geometry ─────────────────────────────────────────────────────────────────

/// Measured tracks → the emitted grid: which columns survive, and where every
/// track edge lands in pixels.
///
/// Format-neutral. Nothing here knows where the measurements came from.
fn build_geometry(mut cols: Vec<Track>, rows: Vec<Track>, nrows: u32, ncols: usize) -> Layout {
    cols.truncate(ncols);
    let vis_cols: Vec<usize> = (0..ncols).filter(|c| !cols[*c].hidden).collect();
    let mut col_left = vec![0.0f32; ncols + 1];
    {
        let mut x = 0.0f32;
        for c in 0..ncols {
            col_left[c] = x;
            if !cols[c].hidden {
                x += cols[c].px;
            }
        }
        col_left[ncols] = x;
    }
    let mut row_top = vec![0.0f32; nrows as usize + 2];
    {
        let mut y = 0.0f32;
        for r in 1..=nrows {
            row_top[r as usize] = y;
            if !rows[r as usize].hidden {
                y += rows[r as usize].px;
            }
        }
        row_top[nrows as usize + 1] = y;
    }
    Layout {
        nrows,
        ncols,
        cols,
        vis_cols,
        rows,
        grid: Grid { col_left, row_top },
    }
}

/// One sticky offset per frozen track, accumulated behind the gutter/header
/// chrome. The offsets ride on the *cells*, never on the `<tr>`: sticky
/// positioning on a table row is not reliably implemented, while on a cell it is.
///
/// Format-neutral.
fn frozen_pane_css(layout: &Layout, frozen: Frozen) -> String {
    let mut css = String::new();
    if !frozen.on() {
        return css;
    }
    let mut x = ROW_HDR_PX;
    for (vi, &c) in layout.vis_cols.iter().enumerate() {
        if c >= frozen.cols {
            break;
        }
        css.push_str(&format!(".fzc{vi}{{left:{}px;}}\n", round(x)));
        x += layout.cols[c].px;
    }
    let mut y = COL_HDR_PX;
    for r in 1..=layout.nrows {
        if r as usize > frozen.rows {
            break;
        }
        if layout.rows[r as usize].hidden {
            continue;
        }
        css.push_str(&format!(".fzr{r}{{top:{}px;}}\n", round(y)));
        y += layout.rows[r as usize].px;
    }
    css
}

// ── emission ─────────────────────────────────────────────────────────────────

/// Opens the document wrappers and the table, then writes the `<colgroup>` and
/// the column-letter header row.
///
/// The wrappers stay open: the drawing overlay is absolutely positioned inside
/// `.xl-grid`, so `render` closes them only once the drawings are emitted.
///
/// Format-neutral.
fn emit_head(
    w: &mut Writer,
    layout: &Layout,
    frozen: Frozen,
    show_lines: bool,
    classes: &mut Classes,
) {
    w.open("div", &attr("class", "xl-doc"));
    w.open("div", &attr("class", "xl-scroll"));
    w.open("div", &attr("class", "xl-grid"));
    let table_class = match (show_lines, frozen.on()) {
        (true, true) => "xl-sheet xl-lines xl-frozen",
        (true, false) => "xl-sheet xl-lines",
        (false, true) => "xl-sheet xl-frozen",
        (false, false) => "xl-sheet",
    };
    w.open("table", &attr("class", table_class));

    w.open("colgroup", "");
    w.void("col", &attr("class", "xl-cg"));
    for &c in &layout.vis_cols {
        let id = classes.widths.id(layout.cols[c].px);
        w.void("col", &attr("class", &format!("xw{id}")));
    }
    w.close();

    w.open("thead", "");
    w.open("tr", "");
    let corner = if frozen.on() {
        "xl-corner xl-fzr fzh xl-fzc fzg"
    } else {
        "xl-corner"
    };
    w.open("th", &attr("class", corner));
    w.close();
    for (vi, &c) in layout.vis_cols.iter().enumerate() {
        let mut cls = String::from("xl-ch");
        if frozen.on() {
            push_class(&mut cls, "xl-fzr fzh");
            if c < frozen.cols {
                push_class(&mut cls, &format!("xl-fzc fzc{vi}"));
            }
        }
        w.open("th", &attr("class", &cls));
        w.text(&col_letter(c));
        w.close();
    }
    w.close();
    w.close();
}

/// The `<tbody>`: one `<tr>` per visible row, with merge spans resolved and the
/// cells pulled from `src`.
///
/// Format-neutral: every cell arrives already resolved, so nothing here knows
/// what a `<c>` is.
fn emit_rows(
    w: &mut Writer,
    layout: &Layout,
    merges: &[Merge],
    src: &mut dyn CellSource,
    frozen: Frozen,
    hl: &mut Marker,
    terms: &Terms,
    classes: &mut Classes,
) {
    let nrows = layout.nrows;
    let ncols = layout.ncols;
    let cols = &layout.cols;
    let vis_cols = &layout.vis_cols;
    let rows = &layout.rows;

    w.open("tbody", "");
    // Reused per row so a wide sheet does not reallocate these every row.
    let mut covered = vec![false; ncols];
    let mut spans: Vec<Option<(u32, usize)>> = vec![None; ncols];

    'rows: for r in 1..=nrows {
        if rows[r as usize].hidden {
            continue;
        }
        // Merge bookkeeping for this row: which columns a span already covers, and
        // where a span starts. Rebuilt per row so nothing has to materialize a
        // covered-cell set for a merge that runs the height of the sheet.
        covered.iter_mut().for_each(|v| *v = false);
        spans.iter_mut().for_each(|v| *v = None);
        for m in merges {
            if r < m.r0 || r > m.r1 {
                continue;
            }
            for c in m.c0..=m.c1.min(ncols - 1) {
                if r == m.ar && c == m.ac {
                    // Spans count *emitted* tracks only: hidden rows/columns are
                    // not in the table, so counting them would push the rest of the
                    // row sideways.
                    let rs = (m.r0..=m.r1)
                        .filter(|rr| !rows[*rr as usize].hidden)
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

        src.row(r);

        let hid = classes.heights.id(rows[r as usize].px);
        let row_frozen = frozen.on() && (r as usize) <= frozen.rows;
        w.open("tr", &attr("class", &format!("xh{hid}")));
        let mut rh_cls = String::from("xl-rh");
        if frozen.on() {
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
            let cell = src.cell(c);

            let mut cls = String::new();
            if let Some(id) = cell.style {
                classes.used.insert(id);
                cls.push_str("xf");
                cls.push_str(&id.to_string());
            }
            if let Some(k) = align_class(cell.align) {
                push_class(&mut cls, k);
            }
            // The one CSS value that comes from a parsed format code rather than
            // from a fixed table, so it is checked here rather than trusted.
            if let Some(col) = cell.color.filter(|c| is_hex_color(c)) {
                let idx = match classes.num_colors.iter().position(|c| *c == col) {
                    Some(i) => i,
                    None => {
                        classes.num_colors.push(col);
                        classes.num_colors.len() - 1
                    }
                };
                push_class(&mut cls, &format!("xnc{idx}"));
            }
            if frozen.on() && (c < frozen.cols || row_frozen) {
                // Frozen cells need an opaque backdrop so scrolled content does not
                // show through. `td.xl-fz` ties with `td.xfN` on specificity and is
                // emitted first, so a document fill still wins.
                push_class(&mut cls, "xl-fz");
                if c < frozen.cols {
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
            // Marks the cell as carrying text. The frame's selection engine keys
            // both the I-beam cursor and the select-vs-pan decision off this: a
            // press on a cell with something in it starts a selection, anywhere
            // else starts a pan (the same rule the PDF text layer follows). CSS
            // cannot ask whether an element has text, and walking the DOM per
            // mousemove over a 6000-cell grid is not free, so the renderer says so.
            if !cell.text.is_empty() {
                push_class(&mut cls, "xl-t");
            }
            let class_attr = if cls.is_empty() {
                String::new()
            } else {
                attr("class", &cls)
            };
            w.open("td", &attrs(&[&class_attr, &rowspan, &colspan]));
            if !cell.text.is_empty() {
                if cell.inner {
                    w.open("span", &attr("class", "xr"));
                }
                // `mark` escapes as it wraps matches, so its output is the only
                // document-derived string that may go in raw.
                let marked = hl.mark(&cell.text, terms);
                w.raw(&marked);
                if cell.inner {
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
}

/// The document stylesheet, assembled once the grid has been walked.
///
/// Format-neutral.
fn collect_css(classes: Classes, frozen_css: &str, styles: &super::styles::Styles) -> String {
    let mut css = String::new();
    css.push_str(&classes.widths.rules("xw", "width"));
    css.push_str(&classes.heights.rules("xh", "height"));
    css.push_str(frozen_css);
    for (i, c) in classes.num_colors.iter().enumerate() {
        css.push_str(&format!("td.xnc{i}{{color:{c};}}\n"));
    }
    // Style rules last: they must beat the base gridline/alignment rules, which
    // they only do on source order.
    css.push_str(&styles.css_block(classes.used));
    css
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
    /// Excel's "General" alignment: it depends on the value, which is why it
    /// travels on the cell instead of in the per-`xf` rule.
    fn align(self) -> Option<Align> {
        match self {
            Kind::Num => Some(Align::Right),
            Kind::Bool | Kind::Err => Some(Align::Center),
            Kind::Text | Kind::Blank => None,
        }
    }
}

/// The class carrying a cell's own alignment. Left is the table's default and
/// needs no rule, and no *value* asks to be justified.
fn align_class(align: Option<Align>) -> Option<&'static str> {
    match align? {
        Align::Right => Some("xl-num"),
        Align::Center => Some("xl-bool"),
        Align::Left | Align::Justify => None,
    }
}

/// The SpreadsheetML side of the cell loop: the `<row>`/`<c>` nodes values are
/// read off, plus the workbook-wide inputs a displayed value depends on.
///
/// Bundled because `Ctx` cannot lend its fields one by one across a call boundary
/// while the rest of it stays borrowed.
struct SheetRows<'a, 'd> {
    row_nodes: &'a [Option<roxmltree::Node<'d, 'd>>],
    rows: &'a [Track],
    cols: &'a [Track],
    merges: &'a [Merge],
    styles: &'a super::styles::Styles,
    sst: &'a [String],
    date1904: bool,
    notes: &'a mut Notes,
    /// Whether any merge's span moved off its stored top-left cell.
    displaced: bool,
    /// The row `row` last positioned on.
    r: u32,
    /// That row's `<c>` nodes by column. Reused so a wide sheet does not
    /// reallocate it every row.
    cells: Vec<Option<roxmltree::Node<'d, 'd>>>,
}

impl CellSource for SheetRows<'_, '_> {
    fn row(&mut self, r: u32) {
        self.r = r;
        self.cells.iter_mut().for_each(|v| *v = None);
        if let Some(rn) = self.row_nodes[r as usize] {
            let mut implied_col = 0usize;
            for cn in elems(rn).filter(|n| n.tag_name().name() == "c") {
                let Some(c) = cell_col(cn, implied_col) else {
                    continue;
                };
                implied_col = c + 1;
                if c < self.cells.len() {
                    self.cells[c] = Some(cn);
                }
            }
        }

        // A merged range stores its value in the top-left cell only. When that cell
        // is in a hidden row or column the span is emitted somewhere else, so the
        // visible anchor borrows the value — otherwise a merged title above a hidden
        // row, or spanning a hidden helper column, renders blank.
        if self.displaced {
            for m in self.merges {
                if r != m.ar || (m.ar == m.r0 && m.ac == m.c0) || self.cells[m.ac].is_some() {
                    continue;
                }
                let Some(anchor) = self.row_nodes[m.r0 as usize] else {
                    continue;
                };
                let mut implied = 0usize;
                for cn in elems(anchor).filter(|n| n.tag_name().name() == "c") {
                    let Some(c) = cell_col(cn, implied) else {
                        continue;
                    };
                    implied = c + 1;
                    if c == m.c0 {
                        self.cells[m.ac] = Some(cn);
                        break;
                    }
                }
            }
        }
    }

    fn cell(&mut self, c: usize) -> Cell {
        let styles = self.styles;
        let node = self.cells[c];
        // A cell states its own xf, or falls back to its row's and then its
        // column's default.
        let style_id = node
            .and_then(|n| attr_u32(n, "s"))
            .or(self.rows[self.r as usize].style)
            .or(self.cols[c].style)
            .unwrap_or(0);
        let cs = styles.get(style_id);
        let (text, kind) = match node {
            Some(n) => cell_text(n, &cs.fmt, self.sst, self.date1904, self.notes),
            None => (String::new(), Kind::Blank),
        };
        let color = node
            .filter(|_| kind == Kind::Num)
            .and_then(numeric_value)
            .and_then(|v| cs.fmt.color(v))
            .map(|c| c.to_string());
        Cell {
            text,
            align: kind.align(),
            style: styles.has_css(style_id).then_some(style_id),
            inner: styles.has_inner(style_id),
            color,
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
    notes: &mut Notes,
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
                    notes.add(
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
            "t" => xml::inner_text(ch, out),
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

/// `<mergeCells>` → the spans of the emitted grid. SpreadsheetML-specific; what
/// it produces is a plain list of rectangles.
fn parse_merges(root: roxmltree::Node<'_, '_>, nrows: u32, ncols: usize) -> Vec<Merge> {
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
    merges
}

/// A merge's normalized `(r0, c0, r1, c1)`, unclamped — rows 1-based, columns
/// 0-based. Used before the sheet's extent is known, to let a merge extend it.
fn merge_bounds(r: &str) -> Option<(u32, usize, u32, usize)> {
    let (a, b) = r.split_once(':')?;
    let (r0, c0) = parse_ref(a)?;
    let (r1, c1) = parse_ref(b)?;
    Some((r0.min(r1), c0.min(c1), r0.max(r1), c0.max(c1)))
}

/// `A1:C3` → a merge clamped to the emitted grid. A range that runs past the
/// emitted bounds (a merge down a whole column) is clipped rather than dropped, so
/// its visible part still spans.
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
    attr_u32(cell, "s").is_some_and(|s| styles.paints(s))
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
    // Strict: a `.rels` this renderer cannot parse means every drawing on the
    // sheet is gone, and the caller turns the error into a note saying so. The
    // lenient read would drop them without a word.
    let rels = opc::read_rels_strict(ctx.zip, sheet_part, ctx.budget)?;
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
    let drels = opc::read_rels(ctx.zip, &dpart, ctx.budget)?;

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
        // No `graphicData` at all: not a frame this renderer can even label.
        let label = descendant(scope, "graphicData")
            .and_then(|g| xml::attr_local(g, "uri"))
            .map(emit::graphic_label);
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
                Media::Placeholder(reason) => emit::placeholder(w, "xl-ph", &style, reason),
            }
        } else if let Some(l) = label {
            emit::placeholder(w, "xl-ph", &style, l);
        }
    }
    if opened {
        w.close();
    }
    if unsupported {
        ctx.notes
            .add("Some drawing shapes are not shown in the preview.");
    }
    Ok(())
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
                emu_to_px(attr_i64(e, "cx")?),
                emu_to_px(attr_i64(e, "cy")?),
            )
        }
        "absoluteAnchor" => {
            let pos = child(anchor, "pos")?;
            let e = ext?;
            (
                emu_to_px(attr_i64(pos, "x")?),
                emu_to_px(attr_i64(pos, "y")?),
                emu_to_px(attr_i64(e, "cx")?),
                emu_to_px(attr_i64(e, "cy")?),
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

/// Body for a sheet that has no grid to show — unreadable, empty, or not a
/// worksheet at all. A sheet has no intrinsic size, so the canvas needs no style.
pub(super) fn error_body(msg: &str) -> String {
    emit::error_doc(BASE_CSS, "xl-doc", "", "xl-empty", msg)
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

#[cfg(test)]
mod tests {
    use super::super::super::drawingml::theme::Theme;
    use super::super::styles::Styles;
    use super::*;

    /// The pipeline phases take a `<worksheet>` root, so a fixture is a document
    /// the caller keeps alive — the nodes borrow it.
    fn ws(body: &str) -> String {
        format!("<worksheet>{body}</worksheet>")
    }

    fn cols_of(specs: &[(f32, bool)]) -> Vec<Track> {
        specs
            .iter()
            .map(|(px, hidden)| Track {
                px: *px,
                hidden: *hidden,
                style: None,
            })
            .collect()
    }

    /// Row tracks the way `row_tracks` builds them: 1-based, with an unused slot
    /// at index 0 and one past the last row.
    fn rows_of(specs: &[(f32, bool)]) -> Vec<Track> {
        let mut rows = vec![Track::new(0.0)];
        rows.extend(cols_of(specs));
        rows.push(Track::new(0.0));
        rows
    }

    /// A grid handed straight to the emitter, so the row loop can be exercised
    /// with no SpreadsheetML in sight.
    struct Fixed {
        /// Rows of cells, 1-based to match the tracks.
        rows: Vec<Vec<Cell>>,
        r: usize,
    }

    impl Fixed {
        fn new(rows: Vec<Vec<Cell>>) -> Fixed {
            Fixed { rows, r: 0 }
        }
    }

    impl CellSource for Fixed {
        fn row(&mut self, r: u32) {
            self.r = r as usize;
        }

        fn cell(&mut self, c: usize) -> Cell {
            self.rows
                .get(self.r.wrapping_sub(1))
                .and_then(|row| row.get(c))
                .cloned()
                .unwrap_or_default()
        }
    }

    fn grid_html(layout: &Layout, merges: &[Merge], src: &mut dyn CellSource) -> String {
        let mut w = Writer::new(1 << 16);
        let mut classes = Classes::default();
        let mut hl = Marker::new();
        emit_rows(
            &mut w,
            layout,
            merges,
            src,
            Frozen { rows: 0, cols: 0 },
            &mut hl,
            &Terms::new(&[]),
            &mut classes,
        );
        w.finish()
    }

    /// One default column is 8.43 digits wide, i.e. 64px after rounding.
    const DEF_COL: f32 = 64.0;

    // ── extent ──────────────────────────────────────────────────────────────

    #[test]
    fn used_extent_counts_cells_that_show_something() {
        let xml = ws("<sheetData>\
             <row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"C1\"><v>2</v></c></row>\
             <row r=\"2\"><c r=\"A2\" s=\"4\"/><c r=\"Z2\" s=\"4\"/></row>\
             </sheetData>");
        let doc = xml::parse(&xml).unwrap();
        let ext = used_extent(doc.root_element(), &Styles::empty());
        // Row 2 is styled but blank and its xf paints nothing, so neither its row
        // nor its column Z reaches the extent.
        assert_eq!((ext.last_row, ext.ncols), (1, 3));
        assert!(!ext.rows_clipped && !ext.cols_clipped);
    }

    #[test]
    fn a_painting_xf_inks_an_otherwise_empty_cell() {
        let styles = Styles::parse(
            "<styleSheet><fills count=\"1\">\
               <fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFFFC000\"/></patternFill></fill>\
             </fills><cellXfs count=\"2\"><xf/><xf fillId=\"0\" applyFill=\"1\"/></cellXfs></styleSheet>",
            &Theme::default(),
        )
        .unwrap();
        let xml = ws("<sheetData><row r=\"3\"><c r=\"B3\" s=\"1\"/></row></sheetData>");
        let doc = xml::parse(&xml).unwrap();
        let ext = used_extent(doc.root_element(), &styles);
        assert_eq!((ext.last_row, ext.ncols), (3, 2));
    }

    #[test]
    fn a_merge_reaching_just_past_the_inked_extent_pulls_it_along() {
        let xml = ws("<sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c></row></sheetData>\
             <mergeCells>\
               <mergeCell ref=\"A1:C6\"/>\
               <mergeCell ref=\"A1:A1048576\"/>\
               <mergeCell ref=\"E10:F12\"/>\
             </mergeCells>");
        let doc = xml::parse(&xml).unwrap();
        let ext = used_extent(doc.root_element(), &Styles::empty());
        // `A1:C6` is a title block and drags the extent out to it. The
        // whole-column merge reaches too far and the detached one starts outside
        // the extent, so neither counts — otherwise the trim would be undone.
        assert_eq!((ext.last_row, ext.ncols), (6, 3));
    }

    #[test]
    fn a_merge_cannot_extend_an_extent_that_does_not_exist() {
        let xml = ws("<sheetData/><mergeCells><mergeCell ref=\"A1:C6\"/></mergeCells>");
        let doc = xml::parse(&xml).unwrap();
        let ext = used_extent(doc.root_element(), &Styles::empty());
        assert_eq!((ext.last_row, ext.ncols), (0, 0));
    }

    #[test]
    fn an_extent_past_the_emission_caps_is_clipped_and_flagged() {
        let over_row = super::super::MAX_ROWS + 1;
        let over_col = super::super::col_letter(super::super::MAX_COLS);
        let xml = ws(&format!(
            "<sheetData>\
               <row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"{over_col}1\"><v>2</v></c></row>\
               <row r=\"{over_row}\"><c r=\"A{over_row}\"><v>3</v></c></row>\
             </sheetData>"
        ));
        let doc = xml::parse(&xml).unwrap();
        let ext = used_extent(doc.root_element(), &Styles::empty());
        assert_eq!(ext.last_row, 1);
        assert_eq!(ext.ncols, 1);
        assert!(ext.rows_clipped && ext.cols_clipped);
    }

    // ── columns ─────────────────────────────────────────────────────────────

    #[test]
    fn parse_cols_spreads_one_definition_over_its_whole_range() {
        let xml = ws("<cols>\
             <col min=\"1\" max=\"2\" width=\"20\" customWidth=\"1\"/>\
             <col min=\"3\" max=\"3\" hidden=\"1\"/>\
             <col min=\"4\" max=\"4\" style=\"7\"/>\
             </cols>");
        let doc = xml::parse(&xml).unwrap();
        let cols = parse_cols(doc.root_element(), chars_to_px(DEFAULT_COL_CHARS));
        assert_eq!(cols.len(), super::super::MAX_COLS);
        // 20 digits * 7px + 5px padding.
        assert_eq!(cols[0].px, 145.0);
        assert_eq!(cols[1].px, 145.0);
        assert!(cols[2].hidden);
        // A hidden column keeps its width; it is the geometry pass that drops it.
        assert_eq!(cols[2].px, DEF_COL);
        assert_eq!(cols[3].style, Some(7));
        // Untouched columns take the sheet default, rounded to a whole pixel.
        assert_eq!(cols[4].px, DEF_COL);
        assert!(!cols[4].hidden && cols[4].style.is_none());
    }

    #[test]
    fn parse_cols_keeps_a_sliver_and_ignores_out_of_range_definitions() {
        let xml = ws(&format!(
            "<cols><col min=\"1\" max=\"1\" width=\"-1\"/>\
               <col min=\"{0}\" max=\"{0}\" width=\"30\"/></cols>",
            super::super::MAX_COLS + 1
        ));
        let doc = xml::parse(&xml).unwrap();
        let cols = parse_cols(doc.root_element(), chars_to_px(DEFAULT_COL_CHARS));
        assert_eq!(cols[0].px, MIN_COL_PX);
        assert_eq!(cols.len(), super::super::MAX_COLS);
        assert_eq!(cols[super::super::MAX_COLS - 1].px, DEF_COL);
    }

    #[test]
    fn a_sheet_default_column_width_applies_to_every_column() {
        let xml = ws("<sheetFormatPr defaultColWidth=\"5\" defaultRowHeight=\"30\"/>");
        let doc = xml::parse(&xml).unwrap();
        let settings = sheet_settings(doc.root_element());
        let cols = parse_cols(doc.root_element(), settings.def_col_px);
        assert_eq!(cols[0].px, 40.0);
        // 30pt at 96dpi.
        assert_eq!(settings.def_row_px, 40.0);
    }

    #[test]
    fn only_a_frozen_pane_state_freezes() {
        for (state, want) in [("frozen", (1, 2)), ("frozenSplit", (1, 2)), ("split", (0, 0))] {
            let xml = ws(&format!(
                "<sheetViews><sheetView showGridLines=\"0\">\
                   <pane xSplit=\"2\" ySplit=\"1\" state=\"{state}\"/></sheetView></sheetViews>"
            ));
            let doc = xml::parse(&xml).unwrap();
            let s = sheet_settings(doc.root_element());
            assert_eq!((s.frozen_rows, s.frozen_cols), want, "state={state}");
            assert!(!s.show_lines);
        }
    }

    // ── geometry ────────────────────────────────────────────────────────────

    fn three_by_three() -> Layout {
        // Column B and row 2 are hidden.
        let rows = rows_of(&[(20.0, false), (30.0, true), (40.0, false)]);
        build_geometry(cols_of(&[(64.0, false), (100.0, true), (50.0, false)]), rows, 3, 3)
    }

    #[test]
    fn build_geometry_gives_a_hidden_track_no_width() {
        let l = three_by_three();
        assert_eq!(l.vis_cols, vec![0, 2]);
        // B occupies no space, so C starts where B did.
        assert_eq!(l.grid.col_left, vec![0.0, 64.0, 64.0, 114.0]);
        // Row 1 at 0, rows 2 and 3 both at 20 (row 2 is hidden), end at 60.
        assert_eq!(l.grid.row_top, vec![0.0, 0.0, 20.0, 20.0, 60.0]);
        // The column table is trimmed to the emitted extent.
        assert_eq!(l.cols.len(), 3);
    }

    #[test]
    fn grid_lookups_past_the_last_track_clamp_to_the_far_edge() {
        let l = three_by_three();
        assert_eq!(l.grid.x(2), 64.0);
        assert_eq!(l.grid.x(99), 114.0);
        assert_eq!(l.grid.y(3), 20.0);
        assert_eq!(l.grid.y(99), 60.0);
    }

    #[test]
    fn frozen_offsets_accumulate_behind_the_gutter_and_skip_hidden_tracks() {
        let l = three_by_three();
        let css = frozen_pane_css(&l, Frozen { rows: 3, cols: 2 });
        // Only column A is inside the split, and it starts past the row gutter.
        assert_eq!(
            css,
            ".fzc0{left:46px;}\n.fzr1{top:21px;}\n.fzr3{top:41px;}\n",
            "{css}"
        );
    }

    #[test]
    fn no_frozen_pane_emits_no_sticky_rules() {
        let l = three_by_three();
        assert!(frozen_pane_css(&l, Frozen { rows: 0, cols: 0 }).is_empty());
    }

    #[test]
    fn the_frozen_split_is_clamped_to_the_grid_and_to_the_sticky_budget() {
        let settings = Settings {
            show_lines: true,
            frozen_rows: MAX_FROZEN + 5,
            frozen_cols: 9,
            def_col_px: DEF_COL,
            def_row_px: 20.0,
        };
        let f = Frozen::clamp(&settings, 100, 4);
        assert_eq!((f.rows, f.cols), (MAX_FROZEN, 4));
        assert!(f.on());
        assert!(!Frozen { rows: 0, cols: 0 }.on());
    }

    // ── emission ────────────────────────────────────────────────────────────

    #[test]
    fn a_cell_paints_exactly_what_it_asks_for() {
        let layout = build_geometry(
            cols_of(&[(64.0, false), (50.0, false), (40.0, false)]),
            rows_of(&[(20.0, false)]),
            1,
            3,
        );
        let mut src = Fixed::new(vec![vec![
            Cell {
                text: "(1,50)".to_string(),
                align: Some(Align::Right),
                style: Some(7),
                color: Some("#ff0000".to_string()),
                ..Default::default()
            },
            // A rotated cell needs the wrapper; a blank one gets no attributes at
            // all, and still gets its `<td>`.
            Cell {
                text: "café".to_string(),
                inner: true,
                ..Default::default()
            },
            Cell::default(),
        ]]);
        assert_eq!(
            grid_html(&layout, &[], &mut src),
            "<tbody><tr class=\"xh0\"><th class=\"xl-rh\">1</th>\
             <td class=\"xf7 xl-num xnc0 xl-t\">(1,50)</td>\
             <td class=\"xl-t\"><span class=\"xr\">café</span></td>\
             <td></td></tr></tbody>"
        );
    }

    #[test]
    fn a_colour_a_style_asks_for_has_to_be_a_colour() {
        let layout = build_geometry(cols_of(&[(64.0, false)]), rows_of(&[(20.0, false)]), 1, 1);
        let mut src = Fixed::new(vec![vec![Cell {
            text: "x".to_string(),
            color: Some("red;}body{display:none".to_string()),
            ..Default::default()
        }]]);
        let html = grid_html(&layout, &[], &mut src);
        assert!(!html.contains("xnc"), "{html}");
    }

    #[test]
    fn a_span_counts_emitted_tracks_and_suppresses_the_cells_it_covers() {
        // Row 1 and column A are hidden, so `A1:C3` is emitted at B2 and spans the
        // two rows and two columns the table actually has.
        let layout = build_geometry(
            cols_of(&[(64.0, true), (50.0, false), (40.0, false)]),
            rows_of(&[(20.0, true), (20.0, false), (20.0, false)]),
            3,
            3,
        );
        let mut merges = vec![Merge {
            r0: 1,
            r1: 3,
            c0: 0,
            c1: 2,
            ar: 1,
            ac: 0,
        }];
        resolve_anchors(&mut merges, &layout.rows, &layout.cols);
        assert_eq!((merges[0].ar, merges[0].ac), (2, 1));

        let mut src = Fixed::new(vec![
            vec![Cell::default(); 3],
            vec![
                Cell::default(),
                Cell {
                    text: "café".to_string(),
                    ..Default::default()
                },
                Cell::default(),
            ],
            vec![Cell::default(); 3],
        ]);
        assert_eq!(
            grid_html(&layout, &merges, &mut src),
            "<tbody>\
             <tr class=\"xh0\"><th class=\"xl-rh\">2</th>\
             <td class=\"xl-t\" rowspan=\"2\" colspan=\"2\">café</td></tr>\
             <tr class=\"xh0\"><th class=\"xl-rh\">3</th></tr>\
             </tbody>"
        );
    }
}
