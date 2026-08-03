//! The format-neutral paragraph model, and its HTML.
//!
//! Between "parse a format's markup" and "write HTML" sits one vocabulary:
//! paragraphs of runs carrying *resolved* presentational values — a size in
//! points, a colour, a hanging indent in px — rather than any one format's
//! spelling of them. A pptx `a:p` and a docx `w:p` disagree about nearly every
//! attribute name and unit and agree about all of this, so resolving the
//! cascade stays with the format and emitting it is written once.
//!
//! Types and emission live together deliberately: the model exists to be
//! rendered, and keeping the two in one file means a new field cannot be added
//! without deciding what it paints.
//!
//! A renderer owns its stylesheet, so the class names arrive in [`HtmlStyle`]
//! and nothing here hard-codes a format's CSS.

use super::drawingml::color::Color;
use super::highlight::{Marker, Terms};
use super::html::{attr, attrs, fmt_pct, fmt_px, pt_to_px, Style, Writer};

/// Line spacing stated as a percentage is relative to a single line of the
/// font, which is taller than the em box. Nothing here measures text, so this is
/// the multiplier that turns "100%" into a CSS `line-height`.
pub const SINGLE_LINE: f32 = 1.2;

// ── the model ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

impl Align {
    fn css(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
            Align::Justify => "justify",
        }
    }
}

/// How tall one line of the paragraph is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineHeight {
    /// Multiple of the font size, i.e. a CSS `line-height` number. A single
    /// line is [`SINGLE_LINE`], which is also the default.
    Multiple(f32),
    /// Exact height in px, whatever the font does.
    Exact(f32),
}

impl Default for LineHeight {
    fn default() -> Self {
        LineHeight::Multiple(SINGLE_LINE)
    }
}

/// Capitalisation applied to a run's text without changing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caps {
    All,
    Small,
}

/// Superscript / subscript. The offset itself is not modelled: no format states
/// a usable one (DrawingML's `baseline` is a percentage of an unstated line
/// height), and CSS `vertical-align` is what every renderer here emits anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Super,
    Sub,
}

/// The bullet or number in front of a paragraph, already resolved to the exact
/// characters it shows — the numbering state, the symbol-font remap and the
/// level's own glyph are all the parser's problem.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListMarker {
    pub label: String,
    pub color: Option<Color>,
    /// CSS font stack, or `None` to take the paragraph's.
    pub font: Option<String>,
    /// Absolute size in points, or `None` to take the paragraph's.
    pub size_pt: Option<f32>,
}

/// A styled span of text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextRun {
    pub text: String,
    /// Absolute, and never inherited: a run that states no size of its own
    /// carries the paragraph's, so the emitter can compare the two and write
    /// nothing when they agree.
    pub size_pt: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// `None` inherits the shape's or page's colour through CSS.
    pub color: Option<Color>,
    /// CSS font stack — already through the substitution table, i.e. safe to
    /// put in a `font-family`.
    pub font: Option<String>,
    pub caps: Option<Caps>,
    pub letter_spacing_pt: f32,
    pub script: Option<Script>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Run {
    Text(TextRun),
    /// A break inside the paragraph (`a:br`, `w:br`).
    Break,
}

/// One paragraph: its box, its marker, and the runs it holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Para {
    pub runs: Vec<Run>,
    /// The paragraph's own font size. It sizes an otherwise empty line and the
    /// marker, and is the size every run is measured against.
    pub size_pt: f32,
    /// `None` leaves the alignment to whatever the box states.
    pub align: Option<Align>,
    /// Indent of the whole paragraph from the box's leading edge, in px.
    pub indent_px: f32,
    /// First-line offset from `indent_px`, in px. Negative is the usual case
    /// for a list: the marker hangs to the left of the text.
    pub first_line_px: f32,
    pub line: LineHeight,
    pub space_before_px: f32,
    pub space_after_px: f32,
    pub marker: Option<ListMarker>,
    pub rtl: bool,
}

// ── html ─────────────────────────────────────────────────────────────────────

/// How a renderer spells the model: its own class names, plus whether lengths
/// ride a CSS variable the frontend can shrink after layout.
pub struct HtmlStyle {
    pub para_class: &'static str,
    /// Class for a paragraph that has a marker — a flex row rather than a
    /// block, see the hanging-indent note in [`emit_para`].
    pub list_class: &'static str,
    pub marker_class: &'static str,
    /// Wraps the runs of a marked paragraph, so they wrap at their own left
    /// edge instead of under the marker.
    pub text_class: &'static str,
    /// Sizes are emitted as `calc(… * var(--af, 1))` so a fitting pass in the
    /// frame can scale the whole box. See [`scaled_px`].
    pub scalable: bool,
}

pub fn emit_paras(w: &mut Writer, paras: &[Para], st: &HtmlStyle, hl: &mut Marker, terms: &Terms) {
    for p in paras {
        if w.is_full() {
            break;
        }
        emit_para(w, p, st, hl, terms);
    }
}

fn emit_para(w: &mut Writer, p: &Para, st: &HtmlStyle, hl: &mut Marker, terms: &Terms) {
    let hang = p.first_line_px;
    let marked = p.marker.is_some();

    let mut s = Style::new();
    s.push_opt("text-align", p.align.map(|a| a.css().to_string()));
    if marked {
        // Hanging indent as a flex row rather than `text-indent`: the marker is
        // a fixed-width first item and the runs are a block that wraps at its
        // own left edge. With `text-indent` a first word wider than what is
        // left of the line moves to the next line *whole* instead of breaking,
        // which leaves the marker sitting alone on a line of its own.
        s.push_opt(
            "margin-left",
            Some((p.indent_px + hang).max(0.0))
                .filter(|v| *v != 0.0)
                .and_then(fmt_px),
        );
    } else {
        s.push_opt(
            "margin-left",
            Some(p.indent_px).filter(|v| *v != 0.0).and_then(fmt_px),
        );
        s.push_opt("text-indent", Some(hang).filter(|v| *v != 0.0).and_then(fmt_px));
    }
    s.push_opt(
        "margin-top",
        Some(p.space_before_px).filter(|v| *v > 0.0).and_then(fmt_px),
    );
    s.push_opt(
        "margin-bottom",
        Some(p.space_after_px).filter(|v| *v > 0.0).and_then(fmt_px),
    );
    s.push_opt("font-size", scaled_px(pt_to_px(p.size_pt), st.scalable));
    match p.line {
        LineHeight::Exact(px) => s.push_opt("line-height", scaled_px(px, st.scalable)),
        // The default costs nothing to state and appears on every paragraph of
        // every deck, so it is left to the stylesheet.
        LineHeight::Multiple(m) => {
            if (m - SINGLE_LINE).abs() > 0.001 {
                s.push_opt("line-height", fmt_pct(m * 100.0));
            }
        }
    }
    if p.rtl {
        s.push("direction", "rtl");
    }
    let class = if marked { st.list_class } else { st.para_class };
    w.open("p", &attrs(&[&attr("class", class), &s.to_attr()]));

    if let Some(m) = p.marker.as_ref() {
        emit_marker(w, m, st, hang);
        w.open("span", &attrs(&[&attr("class", st.text_class)]));
    }

    if p.runs.is_empty() {
        // A non-breaking space keeps the empty line's height.
        w.text("\u{00a0}");
    }
    for r in &p.runs {
        match r {
            Run::Break => w.void("br", ""),
            // A run with no text is not a blank line — only a paragraph with no
            // runs at all is — so it is dropped rather than spaced.
            Run::Text(t) if t.text.is_empty() => {}
            Run::Text(t) => emit_run(w, t, p.size_pt, st, hl, terms),
        }
    }
    if marked {
        w.close(); // text_class
    }
    w.close();
}

/// The marker as the first item of the paragraph's flex row. Its width is the
/// hanging indent, so the text that follows lands exactly on the paragraph's
/// indent without anyone having to measure the glyph.
fn emit_marker(w: &mut Writer, m: &ListMarker, st: &HtmlStyle, hang: f32) {
    let mut s = Style::new();
    if hang < -0.5 {
        s.push_opt("width", fmt_px(-hang));
    } else {
        s.push("padding-right", "0.3em");
    }
    if let Some(c) = m.color.as_ref() {
        s.push("color", &c.css());
    }
    s.push_opt("font-family", m.font.clone());
    s.push_opt("font-size", m.size_pt.and_then(|pt| fmt_px(pt_to_px(pt))));
    w.open("span", &attrs(&[&attr("class", st.marker_class), &s.to_attr()]));
    w.text(&m.label);
    w.close();
}

fn emit_run(
    w: &mut Writer,
    t: &TextRun,
    base_pt: f32,
    st: &HtmlStyle,
    hl: &mut Marker,
    terms: &Terms,
) {
    let mut s = Style::new();
    if (t.size_pt - base_pt).abs() > 0.01 {
        s.push_opt("font-size", scaled_px(pt_to_px(t.size_pt), st.scalable));
    }
    if t.bold {
        s.push("font-weight", "700");
    }
    if t.italic {
        s.push("font-style", "italic");
    }
    let mut deco = String::new();
    if t.underline {
        deco.push_str("underline");
    }
    if t.strike {
        if !deco.is_empty() {
            deco.push(' ');
        }
        deco.push_str("line-through");
    }
    s.push("text-decoration", &deco);
    if let Some(c) = t.color.as_ref() {
        s.push("color", &c.css());
    }
    s.push_opt("font-family", t.font.clone());
    match t.caps {
        Some(Caps::All) => s.push("text-transform", "uppercase"),
        Some(Caps::Small) => s.push("font-variant", "small-caps"),
        None => {}
    }
    s.push_opt(
        "letter-spacing",
        Some(t.letter_spacing_pt)
            .filter(|v| *v != 0.0)
            .map(|v| fmt_px(pt_to_px(v)).unwrap_or_default()),
    );
    match t.script {
        Some(Script::Super) => s.push("vertical-align", "super"),
        Some(Script::Sub) => s.push("vertical-align", "sub"),
        None => {}
    }
    if t.script.is_some() {
        s.push("font-size", "0.65em");
    }
    let html = hl.mark(&t.text, terms);
    if s.is_empty() {
        w.raw(&html);
        return;
    }
    w.open("span", &s.to_attr());
    w.raw(&html);
    w.close();
}

/// A length a fitting pass in the frame can shrink. PowerPoint's own autofit
/// scale is computed against *its* font metrics; with a substituted family the
/// same text needs a different scale, so the box re-fits itself after layout and
/// every size inside it rides on `--af`.
fn scaled_px(px: f32, scalable: bool) -> Option<String> {
    let v = fmt_px(px)?;
    Some(if scalable {
        format!("calc({} * var(--af, 1))", v)
    } else {
        v
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ST: HtmlStyle = HtmlStyle {
        para_class: "p",
        list_class: "p li",
        marker_class: "bu",
        text_class: "tx",
        scalable: false,
    };

    fn run(text: &str) -> Run {
        Run::Text(TextRun {
            text: text.to_string(),
            size_pt: 18.0,
            ..Default::default()
        })
    }

    fn para(runs: Vec<Run>) -> Para {
        Para {
            runs,
            size_pt: 18.0,
            ..Default::default()
        }
    }

    fn html(paras: &[Para], st: &HtmlStyle) -> String {
        let mut w = Writer::new(1 << 16);
        let mut hl = Marker::new();
        emit_paras(&mut w, paras, st, &mut hl, &Terms::new(&[]));
        w.finish()
    }

    #[test]
    fn a_plain_paragraph_states_only_its_size() {
        assert_eq!(
            html(&[para(vec![run("café")])], &ST),
            "<p class=\"p\" style=\"font-size:24px;\">café</p>"
        );
    }

    #[test]
    fn run_formatting_lands_on_a_span_and_defaults_do_not() {
        let mut t = TextRun {
            text: "naïve".to_string(),
            size_pt: 24.0,
            bold: true,
            italic: true,
            underline: true,
            strike: true,
            color: Some(Color::from_rgb(0x00FF00)),
            font: Some("Widget Sans, sans-serif".to_string()),
            caps: Some(Caps::All),
            letter_spacing_pt: 1.5,
            script: Some(Script::Super),
        };
        let out = html(&[para(vec![Run::Text(t.clone())])], &ST);
        assert!(out.contains("font-size:32px;"), "{out}");
        assert!(out.contains("font-weight:700;font-style:italic;"), "{out}");
        assert!(out.contains("text-decoration:underline line-through;"), "{out}");
        assert!(out.contains("color:#00ff00;"), "{out}");
        assert!(out.contains("text-transform:uppercase;"), "{out}");
        assert!(out.contains("letter-spacing:2px;"), "{out}");
        // Super/subscript is an alignment *and* a size.
        assert!(out.contains("vertical-align:super;font-size:0.65em;"), "{out}");

        // A run that states nothing but its (inherited) size needs no span at all.
        t = TextRun {
            text: "café".to_string(),
            size_pt: 18.0,
            ..Default::default()
        };
        assert_eq!(
            html(&[para(vec![Run::Text(t)])], &ST),
            "<p class=\"p\" style=\"font-size:24px;\">café</p>"
        );
    }

    #[test]
    fn a_marker_hangs_in_the_first_line_indent() {
        let p = Para {
            marker: Some(ListMarker {
                label: "1.".to_string(),
                color: Some(Color::from_rgb(0xFF0000)),
                font: Some("Widget Sans, sans-serif".to_string()),
                size_pt: Some(9.0),
            }),
            indent_px: 36.0,
            first_line_px: -36.0,
            ..para(vec![run("café")])
        };
        let out = html(&[p.clone()], &ST);
        // marL + a full hang cancel out, so the block starts at the box edge and
        // the marker occupies the hang.
        assert!(out.starts_with("<p class=\"p li\" style=\"font-size:24px;\">"), "{out}");
        assert!(
            out.contains("<span class=\"bu\" style=\"width:36px;color:#ff0000;\
font-family:Widget Sans, sans-serif;font-size:12px;\">1.</span>"),
            "{out}"
        );
        assert!(out.contains("<span class=\"tx\">café</span>"), "{out}");

        // No hang: the marker cannot be given a width, so it takes a gap.
        let flat = Para {
            first_line_px: 0.0,
            indent_px: 0.0,
            ..p
        };
        assert!(
            html(&[flat], &ST).contains("style=\"padding-right:0.3em;"),
            "no fallback gap"
        );
    }

    #[test]
    fn an_unmarked_paragraph_spells_its_indent_as_margin_and_text_indent() {
        let p = Para {
            indent_px: 48.0,
            first_line_px: -24.0,
            ..para(vec![run("café")])
        };
        assert!(
            html(&[p], &ST).contains("margin-left:48px;text-indent:-24px;"),
            "indent not split"
        );
    }

    #[test]
    fn line_height_states_only_what_differs_from_a_single_line() {
        let with = |line| html(&[Para { line, ..para(vec![run("x")]) }], &ST);
        assert!(!with(LineHeight::default()).contains("line-height"));
        assert!(with(LineHeight::Multiple(1.8)).contains("line-height:180%;"));
        assert!(with(LineHeight::Exact(40.0)).contains("line-height:40px;"));
    }

    #[test]
    fn scalable_sizes_ride_the_fitting_variable() {
        let st = HtmlStyle { scalable: true, ..ST };
        let p = Para {
            line: LineHeight::Exact(30.0),
            ..para(vec![Run::Text(TextRun {
                text: "café".to_string(),
                size_pt: 36.0,
                ..Default::default()
            })])
        };
        let out = html(&[p], &st);
        assert!(out.contains("font-size:calc(24px * var(--af, 1));"), "{out}");
        assert!(out.contains("line-height:calc(30px * var(--af, 1));"), "{out}");
        assert!(out.contains("font-size:calc(48px * var(--af, 1));"), "{out}");
    }

    #[test]
    fn an_empty_paragraph_keeps_its_height_but_an_empty_run_does_not() {
        assert!(html(&[para(vec![])], &ST).contains('\u{00a0}'));
        // A paragraph whose runs are all empty is not a blank line: it holds no
        // text *and* no filler.
        assert_eq!(
            html(&[para(vec![run("")])], &ST),
            "<p class=\"p\" style=\"font-size:24px;\"></p>"
        );
        assert_eq!(
            html(&[para(vec![Run::Break])], &ST),
            "<p class=\"p\" style=\"font-size:24px;\"><br></p>"
        );
    }

    #[test]
    fn alignment_and_direction_reach_the_paragraph_box() {
        let p = Para {
            align: Some(Align::Justify),
            rtl: true,
            space_before_px: 8.0,
            space_after_px: 12.0,
            ..para(vec![run("café")])
        };
        let out = html(&[p], &ST);
        assert!(out.contains("text-align:justify;"), "{out}");
        assert!(out.contains("margin-top:8px;margin-bottom:12px;"), "{out}");
        assert!(out.contains("direction:rtl;"), "{out}");
    }

    #[test]
    fn query_terms_are_marked_inside_the_runs() {
        let mut w = Writer::new(1 << 16);
        let mut hl = Marker::new();
        let terms = Terms::new(&["café".to_string()]);
        emit_paras(&mut w, &[para(vec![run("a café here")])], &ST, &mut hl, &terms);
        let out = w.finish();
        assert!(out.contains("<mark class=\"preview-hl\""), "{out}");
        assert!(hl.best_mark_id().is_some());
    }
}
