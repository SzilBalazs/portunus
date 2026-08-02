//! pptx text bodies: the DrawingML property cascade and paragraph emission.
//!
//! Text properties in a presentation are resolved through a chain, weakest
//! first: `p:defaultTextStyle` (presentation) → the master's `p:txStyles`
//! entry for the shape's kind → the layout placeholder's `a:lstStyle` → the
//! shape's own `a:lstStyle` → the paragraph's `a:pPr` → the run's `a:rPr`.
//! Each link contributes only the properties it actually states, which is why
//! every field below is an `Option` and every merge overwrites nothing it has
//! no value for.
//!
//! Nothing here measures text, so line spacing, autofit scales and bullet
//! hangs are geometric approximations of what PowerPoint computes from real
//! font metrics. See the fidelity notes in the plan.

use super::super::drawingml::color::{parse_color_elem_map, Color};
use super::super::drawingml::theme::{ClrMap, Theme};
use super::super::drawingml::{child_elem, elems};
use super::super::fonts;
use super::super::html::{attr, fmt_px, hundredths_pt_to_px, pt_to_px, Style, Writer};
use super::Ctx;
use roxmltree::Node;

/// Font size used when no link in the cascade states one. PowerPoint's own
/// fallback is the master's body style, which is nearly always present; this is
/// only for documents that ship without one.
const DEFAULT_SZ_PT: f32 = 18.0;

/// `a:lnSpc` percentages are relative to a single line of the font, which is
/// taller than the em box. Without text measurement this is the multiplier that
/// turns "100%" into a CSS `line-height`.
const SINGLE_LINE: f32 = 1.2;

/// Paragraphs per text body, and runs per paragraph. A generated deck can carry
/// thousands; the shape is a fixed box on a 960px canvas, so nothing past this
/// could be read anyway.
const MAX_PARAS: usize = 400;
const MAX_RUNS: usize = 300;

/// Indent levels DrawingML defines (`a:lvl1pPr` … `a:lvl9pPr`).
pub const LEVELS: usize = 9;

// ── properties ───────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct RunProps {
    pub sz_pt: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub color: Option<Color>,
    /// Already through `fonts::css_font_stack`, i.e. safe to put in CSS.
    pub font: Option<String>,
    /// The raw typeface name, kept for the symbol-font remap.
    pub font_raw: Option<String>,
    pub caps: Option<&'static str>,
    pub spc_pt: Option<f32>,
    /// `baseline`: positive is superscript, negative subscript, 0 neither.
    pub baseline: i32,
    /// A run carrying `a:hlinkClick`. Rendered as link-coloured text without an
    /// `href`: the frame has no navigation and no network, so a live link would
    /// be a dead end that looks clickable.
    pub link: bool,
}

#[derive(Clone)]
pub enum Bullet {
    /// Nothing at this link of the cascade — keep whatever the weaker link said.
    Inherit,
    None,
    Char(String),
    /// (`a:buAutoNum@type`, `startAt`)
    AutoNum(String, u32),
}

impl Default for Bullet {
    fn default() -> Self {
        Bullet::Inherit
    }
}

#[derive(Clone, Default)]
pub struct ParaProps {
    pub align: Option<&'static str>,
    pub mar_l: Option<f32>,
    /// First-line offset, normally negative (the bullet hangs left of the text).
    pub indent: Option<f32>,
    pub line_pct: Option<f32>,
    pub line_px: Option<f32>,
    pub before_px: Option<f32>,
    pub after_px: Option<f32>,
    pub bullet: Bullet,
    pub bu_color: Option<Color>,
    pub bu_font: Option<String>,
    pub bu_font_raw: Option<String>,
    pub bu_size_pct: Option<f32>,
    pub rtl: Option<bool>,
    /// `a:defRPr` at this link: the run properties every run in the paragraph
    /// starts from.
    pub run: RunProps,
}

fn truthy(v: &str) -> bool {
    matches!(v, "1" | "true" | "on")
}

fn emu(v: &str) -> Option<f32> {
    v.parse::<i64>()
        .ok()
        .map(super::super::html::emu_to_px)
        .filter(|f| f.is_finite())
}

fn pct(v: &str) -> Option<f32> {
    v.parse::<f32>().ok().map(|p| p / 100_000.0)
}

/// Resolves `+mj-lt` / `+mn-lt` theme font references and hands back both the
/// CSS stack and the raw name (the symbol remap keys off the raw name).
fn typeface(name: &str, theme: &Theme) -> (String, String) {
    let raw = match name {
        "+mj-lt" | "+mj-ea" | "+mj-cs" => theme.major_font.clone(),
        "+mn-lt" | "+mn-ea" | "+mn-cs" => theme.minor_font.clone(),
        other => other.to_string(),
    };
    (fonts::css_font_stack(&raw), raw)
}

/// Merges one `a:rPr` / `a:defRPr` / `a:endParaRPr` into `p`.
pub fn merge_run(p: &mut RunProps, n: Node, theme: &Theme, map: &ClrMap) {
    if let Some(v) = n.attribute("sz") {
        if let Ok(hp) = v.parse::<i64>() {
            let pt = hp as f32 / 100.0;
            if pt > 0.0 && pt < 4000.0 {
                p.sz_pt = Some(pt);
            }
        }
    }
    if let Some(v) = n.attribute("b") {
        p.bold = Some(truthy(v));
    }
    if let Some(v) = n.attribute("i") {
        p.italic = Some(truthy(v));
    }
    if let Some(v) = n.attribute("u") {
        p.underline = Some(v != "none");
    }
    if let Some(v) = n.attribute("strike") {
        p.strike = Some(v != "noStrike");
    }
    if let Some(v) = n.attribute("cap") {
        p.caps = match v {
            "all" => Some("uppercase"),
            "small" => Some("small"),
            _ => None,
        };
    }
    if let Some(v) = n.attribute("spc") {
        if let Ok(hp) = v.parse::<i64>() {
            p.spc_pt = Some(hp as f32 / 100.0);
        }
    }
    if let Some(v) = n.attribute("baseline") {
        if let Ok(b) = v.parse::<f32>() {
            p.baseline = if b > 0.0 {
                1
            } else if b < 0.0 {
                -1
            } else {
                0
            };
        }
    }
    if let Some(sf) = child_elem(n, "solidFill") {
        if let Some(c) = parse_color_elem_map(sf, theme, map, None) {
            p.color = Some(c);
        }
    }
    if let Some(latin) = child_elem(n, "latin") {
        if let Some(tf) = latin.attribute("typeface").filter(|t| !t.is_empty()) {
            let (css, raw) = typeface(tf, theme);
            p.font = Some(css);
            p.font_raw = Some(raw);
        }
    }
    if child_elem(n, "hlinkClick").is_some() {
        p.link = true;
    }
}

/// Merges one `a:pPr` / `a:lvlNpPr` into `p`, including its `a:defRPr`.
pub fn merge_para(p: &mut ParaProps, n: Node, theme: &Theme, map: &ClrMap) {
    if let Some(v) = n.attribute("algn") {
        p.align = match v {
            "l" => Some("left"),
            "ctr" => Some("center"),
            "r" => Some("right"),
            "just" | "justLow" => Some("justify"),
            "dist" | "thaiDist" => Some("justify"),
            _ => None,
        };
    }
    if let Some(v) = n.attribute("marL").and_then(emu) {
        p.mar_l = Some(v);
    }
    if let Some(v) = n.attribute("indent").and_then(emu) {
        p.indent = Some(v);
    }
    if let Some(v) = n.attribute("rtl") {
        p.rtl = Some(truthy(v));
    }
    if let Some(ln) = child_elem(n, "lnSpc") {
        if let Some(sp) = child_elem(ln, "spcPct").and_then(|e| e.attribute("val")).and_then(pct) {
            p.line_pct = Some(sp);
            p.line_px = None;
        } else if let Some(v) = child_elem(ln, "spcPts").and_then(|e| e.attribute("val")) {
            if let Ok(hp) = v.parse::<i64>() {
                p.line_px = Some(hundredths_pt_to_px(hp));
                p.line_pct = None;
            }
        }
    }
    for (tag, slot) in [("spcBef", 0usize), ("spcAft", 1usize)] {
        if let Some(sp) = child_elem(n, tag) {
            // Percentage space-before is a fraction of a line, which needs the
            // resolved font size; it is applied at emit time via `line_px`-free
            // points only, so a percentage is dropped rather than guessed.
            let px = child_elem(sp, "spcPts")
                .and_then(|e| e.attribute("val"))
                .and_then(|v| v.parse::<i64>().ok())
                .map(hundredths_pt_to_px);
            if let Some(px) = px {
                if slot == 0 {
                    p.before_px = Some(px);
                } else {
                    p.after_px = Some(px);
                }
            }
        }
    }
    if child_elem(n, "buNone").is_some() {
        p.bullet = Bullet::None;
    }
    if let Some(bc) = child_elem(n, "buChar") {
        if let Some(ch) = bc.attribute("char").filter(|c| !c.is_empty()) {
            p.bullet = Bullet::Char(ch.chars().take(4).collect());
        }
    }
    if let Some(an) = child_elem(n, "buAutoNum") {
        let ty = an.attribute("type").unwrap_or("arabicPeriod").to_string();
        let start = an
            .attribute("startAt")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1)
            .clamp(1, 32_767);
        p.bullet = Bullet::AutoNum(ty, start);
    }
    if let Some(cf) = child_elem(n, "buClr") {
        if let Some(c) = parse_color_elem_map(cf, theme, map, None) {
            p.bu_color = Some(c);
        }
    }
    if let Some(bf) = child_elem(n, "buFont") {
        if let Some(tf) = bf.attribute("typeface").filter(|t| !t.is_empty()) {
            let (css, raw) = typeface(tf, theme);
            p.bu_font = Some(css);
            p.bu_font_raw = Some(raw);
        }
    }
    if let Some(bs) = child_elem(n, "buSzPct") {
        if let Some(v) = bs.attribute("val").and_then(pct) {
            p.bu_size_pct = Some(v.clamp(0.25, 4.0));
        }
    }
    if let Some(dr) = child_elem(n, "defRPr") {
        merge_run(&mut p.run, dr, theme, map);
    }
}

// ── the cascade ──────────────────────────────────────────────────────────────

/// The chain of `a:lstStyle`-shaped nodes for one shape, weakest link first.
/// Each node is expected to carry `a:lvl1pPr` … `a:lvl9pPr` children.
#[derive(Default)]
pub struct Cascade<'a> {
    pub sources: Vec<Node<'a, 'a>>,
}

impl<'a> Cascade<'a> {
    pub fn push(&mut self, n: Option<Node<'a, 'a>>) {
        if let Some(n) = n {
            self.sources.push(n);
        }
    }

    /// Resolved properties for one indent level, before the paragraph's own
    /// `a:pPr` and the runs' `a:rPr` are applied.
    pub fn level(&self, lvl: usize, theme: &Theme, map: &ClrMap) -> ParaProps {
        let mut p = ParaProps::default();
        for src in &self.sources {
            // Level 1 properties act as the base for deeper levels in Office
            // when a level is skipped, so apply lvl1 first and then the
            // requested level on top of it.
            if lvl > 0 {
                if let Some(n) = lvl_node(*src, 0) {
                    merge_para(&mut p, n, theme, map);
                }
            }
            if let Some(n) = lvl_node(*src, lvl) {
                merge_para(&mut p, n, theme, map);
            }
        }
        p
    }
}

/// `a:lvl{n+1}pPr` child of an `a:lstStyle`-shaped node.
pub fn lvl_node<'a>(src: Node<'a, 'a>, lvl: usize) -> Option<Node<'a, 'a>> {
    let want = format!("lvl{}pPr", lvl.min(LEVELS - 1) + 1);
    elems(src).find(|c| c.tag_name().name() == want)
}

// ── body properties ──────────────────────────────────────────────────────────

pub struct BodyOpts {
    /// Flex `justify-content` for the vertical anchor.
    pub anchor: &'static str,
    /// `a:bodyPr@anchorCtr`: centre the text block horizontally in the box.
    pub anchor_ctr: bool,
    /// Text insets, px: left, top, right, bottom.
    pub ins: (f32, f32, f32, f32),
    pub wrap: bool,
    /// `a:normAutofit@fontScale`, 1.0 when absent.
    pub font_scale: f32,
    /// `a:normAutofit@lnSpcReduction` as a fraction.
    pub ln_reduce: f32,
    /// True for `a:bodyPr@vert` values that rotate the text; only noted, not
    /// rendered.
    pub vertical: bool,
    /// `a:normAutofit` is present: the box shrinks its text to fit. PowerPoint
    /// bakes a `fontScale` computed against *its* font metrics, which are not the
    /// substituted ones, so the frame re-fits the box after layout. See the
    /// autofit pass in `officeBootstrap`.
    pub autofit: bool,
}

impl Default for BodyOpts {
    fn default() -> Self {
        BodyOpts {
            anchor: "flex-start",
            anchor_ctr: false,
            // DrawingML's own defaults: 0.1in left/right, 0.05in top/bottom.
            ins: (9.6, 4.8, 9.6, 4.8),
            wrap: true,
            font_scale: 1.0,
            ln_reduce: 0.0,
            vertical: false,
            autofit: false,
        }
    }
}

/// `a:bodyPr` resolved over the placeholder chain, **most specific first**
/// (shape, layout, master).
///
/// Merged attribute by attribute rather than element-wins: producers write an
/// empty `<a:bodyPr/>` on the shape constantly, and letting it win would reset
/// the anchor to top and the insets to the DrawingML defaults on every inheriting
/// placeholder — which is how a bottom-anchored title ends up drawn over the body
/// text beneath it.
pub fn parse_body_pr(chain: &[Option<Node>]) -> BodyOpts {
    let mut o = BodyOpts::default();
    for n in chain.iter().rev().flatten() {
        merge_body_pr(&mut o, *n);
    }
    o
}

fn merge_body_pr(o: &mut BodyOpts, n: Node) {
    if let Some(v) = n.attribute("anchor") {
        o.anchor = match v {
            "ctr" => "center",
            "b" => "flex-end",
            "just" | "dist" => "space-between",
            _ => "flex-start",
        };
    }
    if let Some(v) = n.attribute("anchorCtr") {
        o.anchor_ctr = truthy(v);
    }
    for (name, slot) in [("lIns", 0), ("tIns", 1), ("rIns", 2), ("bIns", 3)] {
        if let Some(px) = n.attribute(name).and_then(emu) {
            let px = px.clamp(0.0, 400.0);
            match slot {
                0 => o.ins.0 = px,
                1 => o.ins.1 = px,
                2 => o.ins.2 = px,
                _ => o.ins.3 = px,
            }
        }
    }
    if let Some(v) = n.attribute("wrap") {
        o.wrap = v != "none";
    }
    if let Some(v) = n.attribute("vert") {
        o.vertical = v != "horz";
    }
    if let Some(af) = child_elem(n, "normAutofit") {
        o.autofit = true;
        // A stated scale replaces an inherited one; an unstated one means "no
        // shrink was needed at authoring time", not "keep the parent's".
        o.font_scale = af
            .attribute("fontScale")
            .and_then(pct)
            .map(|s| s.clamp(0.25, 1.0))
            .unwrap_or(1.0);
        o.ln_reduce = af
            .attribute("lnSpcReduction")
            .and_then(pct)
            .map(|r| r.clamp(0.0, 0.8))
            .unwrap_or(0.0);
    } else if child_elem(n, "noAutofit").is_some() || child_elem(n, "spAutoFit").is_some() {
        // Stated explicitly at this level: the text overflows (or the box grows,
        // which a fixed preview box cannot do), so no shrink.
        o.autofit = false;
        o.font_scale = 1.0;
        o.ln_reduce = 0.0;
    }
}

// ── emission ─────────────────────────────────────────────────────────────────

/// Emits the absolutely-positioned text box of a shape, then its paragraphs.
/// `w`/`h` are the shape's box in px; the insets come off it.
pub fn emit_body(
    ctx: &mut Ctx,
    w: &mut Writer,
    tb: Node,
    cas: &Cascade,
    opts: &BodyOpts,
) {
    let mut st = Style::new();
    st.push_opt("left", fmt_px(opts.ins.0));
    st.push_opt("top", fmt_px(opts.ins.1));
    st.push_opt("right", fmt_px(opts.ins.2));
    st.push_opt("bottom", fmt_px(opts.ins.3));
    st.push("justify-content", opts.anchor);
    if opts.anchor_ctr {
        st.push("align-items", "center");
    }
    if !opts.wrap {
        st.push("white-space", "pre");
    }
    // `data-af` is what the frame's autofit pass looks for; the paragraph sizes it
    // scales are written as `calc(… * var(--af,1))`.
    let af = if opts.autofit {
        attr("data-af", "1")
    } else {
        String::new()
    };
    w.open(
        "div",
        &super::super::html::attrs(&[&attr("class", "pp-tb"), &af, &st.to_attr()]),
    );
    // The paragraphs live in their own block so the autofit pass can measure how
    // tall the text actually is: a bottom-anchored box that overflows does so
    // *upwards*, where `scrollHeight` cannot see it.
    w.open("div", &super::super::html::attrs(&[&attr("class", "pp-tbi")]));
    emit_paras(ctx, w, tb, cas, opts);
    w.close();
    w.close();
}

/// Emits just the paragraphs of a text body — used directly by table cells,
/// which supply their own box.
pub fn emit_paras(ctx: &mut Ctx, w: &mut Writer, tb: Node, cas: &Cascade, opts: &BodyOpts) {
    let theme = ctx.theme;
    let map = ctx.clr_map;
    // One counter per indent level for `a:buAutoNum`; a deeper level restarts
    // whenever a shallower paragraph intervenes, which is how PowerPoint
    // numbers nested lists.
    let mut counters = [0u32; LEVELS];
    let mut n_paras = 0usize;
    for para in elems(tb) {
        if para.tag_name().name() != "p" {
            continue;
        }
        if w.is_full() || n_paras >= MAX_PARAS {
            break;
        }
        n_paras += 1;

        let ppr = child_elem(para, "pPr");
        let lvl = ppr
            .and_then(|n| n.attribute("lvl"))
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0)
            .min(LEVELS - 1);
        let mut pp = cas.level(lvl, theme, &map);
        if let Some(n) = ppr {
            merge_para(&mut pp, n, theme, &map);
        }

        // Runs first, so the paragraph's own font size can be the first run's
        // (it sizes the bullet and an otherwise empty line).
        let runs: Vec<Node> = elems(para)
            .filter(|c| matches!(c.tag_name().name(), "r" | "br" | "fld"))
            .take(MAX_RUNS)
            .collect();
        let first_rpr = runs
            .iter()
            .find(|r| r.tag_name().name() == "r")
            .and_then(|r| child_elem(*r, "rPr"));
        let mut base = pp.run.clone();
        if let Some(n) = first_rpr {
            merge_run(&mut base, n, theme, &map);
        }
        if runs.is_empty() {
            // An empty paragraph is a spacer whose height comes from
            // `a:endParaRPr`.
            if let Some(n) = child_elem(para, "endParaRPr") {
                merge_run(&mut base, n, theme, &map);
            }
        }
        let base_pt = base.sz_pt.unwrap_or(DEFAULT_SZ_PT) * opts.font_scale;

        // The bullet label is resolved before the paragraph box is written, because
        // whether there is one decides how the indent is spelled. A blank line in a
        // bulleted list carries no bullet in PowerPoint and does not consume a
        // number either, so an empty paragraph asks for neither.
        let label = if runs.iter().any(|r| any_text(*r)) {
            bullet_label(&pp, lvl, &mut counters)
        } else {
            None
        };
        let hang = pp.indent.unwrap_or(0.0);
        let mar_l = pp.mar_l.unwrap_or(0.0);

        let mut st = Style::new();
        st.push_opt("text-align", pp.align.map(str::to_string));
        if label.is_some() {
            // Hanging indent as a flex row rather than `text-indent`: the bullet is
            // a fixed-width first item and the runs are a block that wraps at its
            // own left edge. With `text-indent` a first word wider than what is
            // left of the line moves to the next line *whole* instead of breaking,
            // which leaves the bullet sitting alone on a line of its own.
            st.push_opt(
                "margin-left",
                Some((mar_l + hang).max(0.0)).filter(|v| *v != 0.0).and_then(fmt_px),
            );
        } else {
            st.push_opt("margin-left", pp.mar_l.filter(|v| *v != 0.0).and_then(fmt_px));
            st.push_opt("text-indent", pp.indent.filter(|v| *v != 0.0).and_then(fmt_px));
        }
        st.push_opt("margin-top", pp.before_px.filter(|v| *v > 0.0).and_then(fmt_px));
        st.push_opt("margin-bottom", pp.after_px.filter(|v| *v > 0.0).and_then(fmt_px));
        st.push_opt("font-size", scaled_px(pt_to_px(base_pt), opts.autofit));
        if let Some(px) = pp.line_px {
            st.push_opt("line-height", scaled_px(px, opts.autofit));
        } else {
            let mult = pp.line_pct.unwrap_or(1.0) * SINGLE_LINE * (1.0 - opts.ln_reduce);
            if (mult - SINGLE_LINE).abs() > 0.001 {
                st.push_opt("line-height", super::super::html::fmt_pct(mult * 100.0));
            }
        }
        if pp.rtl == Some(true) {
            st.push("direction", "rtl");
        }
        let class = if label.is_some() { "pp-p pp-li" } else { "pp-p" };
        w.open("p", &super::super::html::attrs(&[&attr("class", class), &st.to_attr()]));

        if let Some(label) = label.as_deref() {
            emit_bullet(w, &pp, label, hang, base_pt);
            w.open("span", &super::super::html::attrs(&[&attr("class", "pp-tx")]));
        }

        if runs.is_empty() {
            // A non-breaking space keeps the empty line's height.
            w.text("\u{00a0}");
        }
        for r in runs {
            match r.tag_name().name() {
                "br" => w.void("br", ""),
                // A field (slide number, date) renders its cached text; the live
                // value is not computable here.
                "r" | "fld" => {
                    let mut rp = pp.run.clone();
                    if let Some(n) = child_elem(r, "rPr") {
                        merge_run(&mut rp, n, theme, &map);
                    }
                    let text = run_text(r, &rp);
                    if text.is_empty() {
                        continue;
                    }
                    emit_run(ctx, w, &text, &rp, base_pt, opts.font_scale, opts.autofit);
                }
                _ => {}
            }
        }
        if label.is_some() {
            w.close(); // pp-tx
        }
        w.close();
    }
}

/// A length the frame's autofit pass can shrink. PowerPoint's own `fontScale` is
/// computed against *its* font metrics; with a substituted family the same text
/// needs a different scale, so the box re-fits itself after layout and every size
/// inside it rides on `--af`.
fn scaled_px(px: f32, autofit: bool) -> Option<String> {
    let v = fmt_px(px)?;
    Some(if autofit {
        format!("calc({} * var(--af, 1))", v)
    } else {
        v
    })
}

/// Whether a run carries any non-whitespace text. Checked before the bullet is
/// emitted, so it must not allocate the text to find out.
fn any_text(r: Node) -> bool {
    if !matches!(r.tag_name().name(), "r" | "fld") {
        return false;
    }
    elems(r)
        .filter(|t| t.tag_name().name() == "t")
        .flat_map(|t| t.descendants())
        .any(|d| d.text().map(|x| !x.trim().is_empty()).unwrap_or(false))
}

/// Concatenated `a:t` text of a run, with symbol-font code points mapped to
/// their Unicode equivalents so Wingdings bullets and Symbol Greek do not
/// render as boxes.
fn run_text(r: Node, rp: &RunProps) -> String {
    let mut s = String::new();
    for t in elems(r) {
        if t.tag_name().name() == "t" {
            for d in t.descendants() {
                if d.is_text() {
                    if let Some(x) = d.text() {
                        s.push_str(x);
                    }
                }
            }
        }
    }
    let symbol = rp
        .font_raw
        .as_deref()
        .map(fonts::is_symbol_font)
        .unwrap_or(false);
    if !symbol {
        return s;
    }
    let font = rp.font_raw.as_deref().unwrap_or("");
    s.chars()
        .map(|c| fonts::remap(font, c).unwrap_or(c))
        .collect()
}

fn emit_run(
    ctx: &mut Ctx,
    w: &mut Writer,
    text: &str,
    rp: &RunProps,
    base_pt: f32,
    scale: f32,
    autofit: bool,
) {
    let mut st = Style::new();
    let pt = rp.sz_pt.map(|v| v * scale);
    if let Some(pt) = pt.filter(|p| (*p - base_pt).abs() > 0.01) {
        st.push_opt("font-size", scaled_px(pt_to_px(pt), autofit));
    }
    if rp.bold == Some(true) {
        st.push("font-weight", "700");
    }
    if rp.italic == Some(true) {
        st.push("font-style", "italic");
    }
    let mut deco = String::new();
    if rp.underline == Some(true) || rp.link {
        deco.push_str("underline");
    }
    if rp.strike == Some(true) {
        if !deco.is_empty() {
            deco.push(' ');
        }
        deco.push_str("line-through");
    }
    st.push("text-decoration", &deco);
    match rp.color.as_ref() {
        Some(c) => st.push("color", &c.css()),
        // An unstyled hyperlink takes the theme's link colour; anything else
        // inherits the shape's.
        None if rp.link => st.push(
            "color",
            &Color::from_rgb(ctx.theme.color(
                super::super::drawingml::theme::SchemeSlot::Hlink,
            ))
            .css(),
        ),
        None => {}
    }
    st.push_opt("font-family", rp.font.clone());
    match rp.caps {
        Some("uppercase") => st.push("text-transform", "uppercase"),
        Some("small") => st.push("font-variant", "small-caps"),
        _ => {}
    }
    st.push_opt("letter-spacing", rp.spc_pt.filter(|v| *v != 0.0).map(|v| {
        fmt_px(pt_to_px(v)).unwrap_or_default()
    }));
    match rp.baseline {
        1 => st.push("vertical-align", "super"),
        -1 => st.push("vertical-align", "sub"),
        _ => {}
    }
    if rp.baseline != 0 {
        st.push("font-size", "0.65em");
    }
    let marked = ctx.marker.mark(text, ctx.terms);
    if st.is_empty() {
        w.raw(&marked);
        return;
    }
    w.open("span", &st.to_attr());
    w.raw(&marked);
    w.close();
}

/// The bullet glyph or number, as an inline-block whose width is the paragraph's
/// hanging indent — so the text after it starts exactly at `marL`, without
/// measuring the glyph.
/// The bullet this paragraph shows, advancing the auto-number counters. `None` for
/// an unbulleted paragraph — which still resets the counters, because a plain
/// paragraph between two numbered ones restarts the numbering in PowerPoint.
fn bullet_label(pp: &ParaProps, lvl: usize, counters: &mut [u32; LEVELS]) -> Option<String> {
    let label = match &pp.bullet {
        Bullet::Inherit | Bullet::None => {
            counters[lvl] = 0;
            return None;
        }
        Bullet::Char(c) => {
            let raw = pp.bu_font_raw.as_deref().unwrap_or("");
            if fonts::is_symbol_font(raw) {
                c.chars().map(|ch| fonts::remap(raw, ch).unwrap_or(ch)).collect()
            } else {
                c.clone()
            }
        }
        Bullet::AutoNum(ty, start) => {
            let n = if counters[lvl] == 0 {
                *start
            } else {
                counters[lvl].saturating_add(1)
            };
            counters[lvl] = n;
            for c in counters.iter_mut().skip(lvl + 1) {
                *c = 0;
            }
            auto_num(ty, n)
        }
    };
    if let Bullet::Char(_) = pp.bullet {
        counters[lvl] = 0;
    }
    for c in counters.iter_mut().skip(lvl + 1) {
        *c = 0;
    }
    Some(label)
}

/// The bullet as the first item of the paragraph's flex row. Its width is the
/// hanging indent, so the text that follows lands exactly on `marL` without
/// anyone having to measure the glyph.
fn emit_bullet(w: &mut Writer, pp: &ParaProps, label: &str, hang: f32, base_pt: f32) {
    let mut st = Style::new();
    if hang < -0.5 {
        st.push_opt("width", fmt_px(-hang));
    } else {
        st.push("padding-right", "0.3em");
    }
    if let Some(c) = pp.bu_color.as_ref() {
        st.push("color", &c.css());
    }
    // A symbol bullet font is only useful for the glyph the remap could not
    // resolve; the remapped glyph reads better in the text font.
    if let Some(f) = pp.bu_font.as_ref() {
        if !fonts::is_symbol_font(pp.bu_font_raw.as_deref().unwrap_or("")) {
            st.push("font-family", f);
        }
    }
    if let Some(p) = pp.bu_size_pct {
        st.push_opt("font-size", fmt_px(pt_to_px(base_pt * p)));
    }
    w.open("span", &super::super::html::attrs(&[&attr("class", "pp-bu"), &st.to_attr()]));
    w.text(label);
    w.close();
}

/// `a:buAutoNum@type` → the rendered label. Unknown types fall back to the
/// arabic-period form rather than dropping the number.
fn auto_num(ty: &str, n: u32) -> String {
    let (body, wrap) = match ty {
        t if t.starts_with("alphaLc") => (alpha(n, false), t),
        t if t.starts_with("alphaUc") => (alpha(n, true), t),
        t if t.starts_with("romanLc") => (roman(n, false), t),
        t if t.starts_with("romanUc") => (roman(n, true), t),
        t => (n.to_string(), t),
    };
    if wrap.ends_with("ParenBoth") {
        format!("({body})")
    } else if wrap.ends_with("ParenR") {
        format!("{body})")
    } else if wrap.ends_with("Period") || wrap.ends_with("PeriodOne") {
        format!("{body}.")
    } else if wrap.ends_with("Dash") {
        format!("- {body}")
    } else {
        format!("{body}.")
    }
}

fn alpha(n: u32, upper: bool) -> String {
    let mut n = n.max(1);
    let base = if upper { b'A' } else { b'a' };
    let mut out = Vec::new();
    while n > 0 {
        let rem = ((n - 1) % 26) as u8;
        out.push(base + rem);
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn roman(n: u32, upper: bool) -> String {
    const T: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut n = n.min(3999);
    let mut out = String::new();
    for (v, s) in T {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    if upper {
        out.to_uppercase()
    } else {
        out
    }
}

/// Flat text of a text body, for slide titles and the section list.
pub fn plain_text(tb: Node, limit: usize) -> String {
    let mut out = String::new();
    for para in elems(tb) {
        if para.tag_name().name() != "p" {
            continue;
        }
        let mut line = String::new();
        for r in elems(para) {
            match r.tag_name().name() {
                "r" | "fld" => {
                    for t in elems(r) {
                        if t.tag_name().name() == "t" {
                            for d in t.descendants() {
                                if d.is_text() {
                                    if let Some(x) = d.text() {
                                        line.push_str(x);
                                    }
                                }
                            }
                        }
                    }
                }
                "br" => line.push(' '),
                _ => {}
            }
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(line);
        if out.chars().count() >= limit {
            break;
        }
    }
    let clipped = out.chars().count() > limit;
    let mut s: String = out.chars().take(limit).collect();
    if clipped {
        s = s.trim_end().to_string();
        s.push('…');
    }
    s.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_num_formats_cover_the_common_types() {
        assert_eq!(auto_num("arabicPeriod", 3), "3.");
        assert_eq!(auto_num("arabicParenR", 3), "3)");
        assert_eq!(auto_num("alphaLcParenBoth", 2), "(b)");
        assert_eq!(auto_num("alphaUcPeriod", 27), "AA.");
        assert_eq!(auto_num("romanUcPeriod", 4), "IV.");
        assert_eq!(auto_num("romanLcPeriod", 9), "ix.");
        // Unknown types still number rather than dropping the marker.
        assert_eq!(auto_num("circleNumDbPlain", 5), "5.");
    }

    #[test]
    fn cascade_applies_level_one_under_deeper_levels() {
        let xml = "<lstStyle xmlns='a'><lvl1pPr algn='ctr'><defRPr sz='2000'/></lvl1pPr>\
                   <lvl2pPr marL='914400'/></lstStyle>";
        let doc = roxmltree::Document::parse(xml).unwrap();
        let mut cas = Cascade::default();
        cas.push(Some(doc.root_element()));
        let theme = Theme::default();
        let map = ClrMap::default();
        let p = cas.level(1, &theme, &map);
        // lvl2 states only marL, so alignment and size come from lvl1.
        assert_eq!(p.align, Some("center"));
        assert_eq!(p.run.sz_pt, Some(20.0));
        assert_eq!(p.mar_l.map(|v| v.round()), Some(96.0));
    }

    #[test]
    fn plain_text_joins_paragraphs_and_clips() {
        let xml = "<txBody xmlns='a'><p><r><t>café</t></r></p><p><r><t>naïve</t></r></p></txBody>";
        let doc = roxmltree::Document::parse(xml).unwrap();
        assert_eq!(plain_text(doc.root_element(), 64), "café naïve");
        assert_eq!(plain_text(doc.root_element(), 5), "café…");
    }
}
