//! Shape geometry: the EMU→px scale, `a:xfrm` parsing, group-transform
//! composition, and as much of `a:prstGeom` as CSS can express.

use crate::office::drawingml::{child_elem, elems};
use crate::office::html::{fmt_pct, fmt_px, Style};
use crate::office::xml;

/// 914400 EMU per inch ÷ 96 px per inch. `html::emu_to_px` is the f32 twin of
/// this; geometry composition needs f64 because a group chain multiplies scale
/// factors and f32 drifts visibly after three levels.
pub const EMU_PER_PX: f64 = 9525.0;

/// `a:rot`, like every other DrawingML angle, is 60000ths of a degree.
const ANG_PER_DEG: f64 = 60_000.0;

/// Deepest group nesting that will be composed. Slide XML is untrusted and a
/// group tree can be arbitrarily deep (or, with a malformed part, effectively
/// unbounded); past this the transform is truncated rather than walked.
pub const MAX_GROUP_DEPTH: usize = 16;

pub fn emu_px(emu: i64) -> f64 {
    emu as f64 / EMU_PER_PX
}

/// A resolved shape transform, in **px** — the unit the renderer emits, so no
/// caller has to remember which side of the conversion it is on. Composition is
/// scale-invariant, so mixing units would be silently wrong rather than obviously
/// wrong.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Xf {
    pub x: f64,
    pub y: f64,
    pub cx: f64,
    pub cy: f64,
    /// Degrees, clockwise (the DrawingML direction, which is also the CSS one).
    pub rot: f64,
    pub flip_h: bool,
    pub flip_v: bool,
}

/// A group's transform plus the child coordinate space it establishes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroupXf {
    pub xf: Xf,
    /// `a:chOff` in px.
    pub ch_off: (f64, f64),
    /// `a:chExt` in px.
    pub ch_ext: (f64, f64),
}

impl Xf {
    /// Map a child transform out of a group's child coordinate space and into the
    /// group's parent space.
    ///
    /// A group's `a:xfrm` carries two rectangles: `a:off`/`a:ext` is where the
    /// group sits in *its* parent, and `a:chOff`/`a:chExt` is the coordinate
    /// system its children were authored in. The two are unrelated numbers — a
    /// group can be 2in wide with a 10in child space — so a child must be
    /// translated by `-chOff`, scaled by `ext/chExt`, then translated by `off`.
    ///
    /// `chExt` of zero is the trap: dividing by it yields a NaN that propagates
    /// into every coordinate of every descendant and then silently deletes the
    /// CSS declarations that carry them. Degenerate child extents fall back to
    /// scale 1.
    pub fn compose(group: &Xf, ch_off: (f64, f64), ch_ext: (f64, f64), child: &Xf) -> Xf {
        let sx = safe_scale(group.cx, ch_ext.0);
        let sy = safe_scale(group.cy, ch_ext.1);
        Xf {
            x: fin(group.x + (child.x - ch_off.0) * sx),
            y: fin(group.y + (child.y - ch_off.1) * sy),
            cx: fin(child.cx * sx),
            cy: fin(child.cy * sy),
            // The group's own rotation is added to the child's rather than
            // rotating the child's *position* about the group centre. Rotating
            // positions is what Office does; doing it here would need the group
            // centre threaded through every level, and the visible difference
            // only shows on rotated groups, which are rare in practice.
            rot: fin(group.rot + child.rot),
            flip_h: group.flip_h ^ child.flip_h,
            flip_v: group.flip_v ^ child.flip_v,
        }
    }

    /// Compose a chain of groups, outermost first, around `child`. Chains longer
    /// than [`MAX_GROUP_DEPTH`] are truncated to their outermost levels: the deep
    /// child ends up mispositioned, which is strictly better than unbounded work
    /// on a hostile part.
    pub fn compose_nested(groups: &[GroupXf], child: &Xf) -> Xf {
        let mut frame: Option<GroupXf> = None;
        for g in groups.iter().take(MAX_GROUP_DEPTH) {
            let mapped = match &frame {
                // The outermost group is already in slide coordinates.
                None => g.xf,
                Some(p) => Xf::compose(&p.xf, p.ch_off, p.ch_ext, &g.xf),
            };
            frame = Some(GroupXf {
                xf: mapped,
                ch_off: g.ch_off,
                ch_ext: g.ch_ext,
            });
        }
        match frame {
            None => *child,
            Some(f) => Xf::compose(&f.xf, f.ch_off, f.ch_ext, child),
        }
    }

    /// Absolute placement for a positioned box. Rotation and flips ride on one
    /// `transform` so they compose about the box centre.
    pub fn css(&self) -> String {
        let mut s = Style::new();
        s.push_opt("left", fmt_px(self.x as f32));
        s.push_opt("top", fmt_px(self.y as f32));
        s.push_opt("width", fmt_px(self.cx as f32));
        s.push_opt("height", fmt_px(self.cy as f32));
        let mut tf = String::new();
        if self.rot.is_finite() && self.rot.abs() > 0.01 {
            tf.push_str(&format!("rotate({:.2}deg)", self.rot));
        }
        if self.flip_h || self.flip_v {
            if !tf.is_empty() {
                tf.push(' ');
            }
            tf.push_str(&format!(
                "scale({}, {})",
                if self.flip_h { -1 } else { 1 },
                if self.flip_v { -1 } else { 1 }
            ));
        }
        s.push("transform", &tf);
        s.css().to_string()
    }
}

/// `group.cx / ch_ext` with the zero and non-finite cases pinned to 1.
fn safe_scale(extent: f64, child_extent: f64) -> f64 {
    if !extent.is_finite() || !child_extent.is_finite() || child_extent.abs() < f64::EPSILON {
        return 1.0;
    }
    let s = extent / child_extent;
    if s.is_finite() {
        s
    } else {
        1.0
    }
}

fn fin(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

/// Parse an `a:xfrm`, either given directly or found among `node`'s direct
/// children (`a:spPr`, `a:grpSpPr`, `p:xfrm` on a graphic frame).
///
/// `a:ext` is required: a transform with no extent cannot place a box, and
/// guessing a size would put a stray 0×0 element on the slide. A missing `a:off`
/// defaults to the origin, which is what producers mean when they omit it.
pub fn parse_xfrm(node: roxmltree::Node<'_, '_>) -> Option<Xf> {
    let xfrm = if node.tag_name().name() == "xfrm" {
        node
    } else {
        elems(node).find(|n| n.tag_name().name() == "xfrm")?
    };
    let ext = child_elem(xfrm, "ext")?;
    let cx = emu_attr(ext, "cx")?;
    let cy = emu_attr(ext, "cy")?;
    let (x, y) = match child_elem(xfrm, "off") {
        Some(off) => (
            emu_attr(off, "x").unwrap_or(0.0),
            emu_attr(off, "y").unwrap_or(0.0),
        ),
        None => (0.0, 0.0),
    };
    Some(Xf {
        x,
        y,
        cx,
        cy,
        rot: xml::attr_local(xfrm, "rot")
            .and_then(|v| v.trim().parse::<i64>().ok())
            .map(|r| r as f64 / ANG_PER_DEG)
            .unwrap_or(0.0),
        flip_h: bool_attr(xfrm, "flipH"),
        flip_v: bool_attr(xfrm, "flipV"),
    })
}

/// As [`parse_xfrm`], plus the `a:chOff`/`a:chExt` child space. A group whose
/// xfrm omits them keeps `chOff = off` and `chExt = ext`, i.e. an identity
/// mapping, which is how a group with no rescaling is written.
pub fn parse_group_xfrm(node: roxmltree::Node<'_, '_>) -> Option<GroupXf> {
    let xfrm = if node.tag_name().name() == "xfrm" {
        node
    } else {
        elems(node).find(|n| n.tag_name().name() == "xfrm")?
    };
    let xf = parse_xfrm(xfrm)?;
    let ch_off = match child_elem(xfrm, "chOff") {
        Some(n) => (
            emu_attr(n, "x").unwrap_or(0.0),
            emu_attr(n, "y").unwrap_or(0.0),
        ),
        None => (xf.x, xf.y),
    };
    let ch_ext = match child_elem(xfrm, "chExt") {
        Some(n) => (
            emu_attr(n, "cx").unwrap_or(xf.cx),
            emu_attr(n, "cy").unwrap_or(xf.cy),
        ),
        None => (xf.cx, xf.cy),
    };
    Some(GroupXf {
        xf,
        ch_off,
        ch_ext,
    })
}

fn emu_attr(node: roxmltree::Node<'_, '_>, name: &str) -> Option<f64> {
    let v = xml::attr_local(node, name)?.trim().parse::<i64>().ok()?;
    Some(emu_px(v))
}

fn bool_attr(node: roxmltree::Node<'_, '_>, name: &str) -> bool {
    matches!(xml::attr_local(node, name), Some("1") | Some("true"))
}

// ── preset geometry ──────────────────────────────────────────────────────────

/// A shape outline reduced to what CSS can draw.
#[derive(Debug, Clone, PartialEq)]
pub enum Geom {
    Rect,
    /// Corner radius as a percentage of the *shorter* side, which is how
    /// DrawingML's `adj` is defined. See [`round_radius_px`].
    RoundRect {
        r_pct: f64,
    },
    Ellipse,
    /// `clip-path: polygon()` vertices in percent of the box.
    Poly(Vec<(f64, f64)>),
    /// The preset (or `custGeom`) has no CSS equivalent and is drawn as a plain
    /// rectangle. Carries the geometry name so the caller can record a fidelity
    /// note.
    Fallback(String),
}

/// Parse `a:prstGeom`/`a:custGeom`, given directly or found among `node`'s direct
/// children. A shape with no geometry element (pictures, text boxes) is a
/// rectangle.
pub fn parse_geom(node: roxmltree::Node<'_, '_>) -> Geom {
    let local = node.tag_name().name();
    let geom = if local == "prstGeom" || local == "custGeom" {
        Some(node)
    } else {
        elems(node).find(|n| {
            matches!(n.tag_name().name(), "prstGeom" | "custGeom")
        })
    };
    let Some(geom) = geom else { return Geom::Rect };
    // A custom path is a Bézier program; approximating it would be worse than
    // admitting the box.
    if geom.tag_name().name() == "custGeom" {
        return Geom::Fallback("custGeom".to_string());
    }
    let Some(prst) = xml::attr_local(geom, "prst") else {
        return Geom::Rect;
    };
    preset_geom(prst, geom)
}

fn preset_geom(prst: &str, geom: roxmltree::Node<'_, '_>) -> Geom {
    // Tier 1: exact in CSS.
    match prst {
        "rect" | "flowChartProcess" => return Geom::Rect,
        // `line`/`straightConnector1` shapes carry a near-zero extent on one
        // axis, so the plain box plus the shape's own `a:ln` border draws them.
        "line" | "straightConnector1" => return Geom::Rect,
        // The outer silhouette of a bevel is a rectangle; the raised facets are
        // not reproduced.
        "bevel" => return Geom::Rect,
        "roundRect" => {
            return Geom::RoundRect {
                r_pct: round_rect_adj(geom),
            }
        }
        "ellipse" | "circle" | "flowChartConnector" => return Geom::Ellipse,
        _ => {}
    }
    // Tier 2: fixed canonical polygons in percent, so they scale with the box.
    // The presets' `a:avLst` adjust handles are deliberately ignored (except
    // roundRect's, above): honouring them means reimplementing each preset's
    // guide formulas, and an unadjusted star reads correctly at preview scale.
    if let Some(pts) = preset_polygon(prst) {
        return Geom::Poly(pts.to_vec());
    }
    // Tier 3: ~170 remaining presets — callouts, banners, snipped/rounded corner
    // variants, math shapes, the rest of the arrows and flowchart symbols.
    Geom::Fallback(prst.to_string())
}

/// Vertices in percent, clockwise from the top. Hand-chosen to match the preset's
/// default proportions, not derived from its guide formulas.
fn preset_polygon(prst: &str) -> Option<&'static [(f64, f64)]> {
    Some(match prst {
        "triangle" => &[(50.0, 0.0), (100.0, 100.0), (0.0, 100.0)],
        "rtTriangle" => &[(0.0, 0.0), (100.0, 100.0), (0.0, 100.0)],
        "diamond" | "flowChartDecision" => {
            &[(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)]
        }
        "parallelogram" => &[(25.0, 0.0), (100.0, 0.0), (75.0, 100.0), (0.0, 100.0)],
        "trapezoid" => &[(25.0, 0.0), (75.0, 0.0), (100.0, 100.0), (0.0, 100.0)],
        "pentagon" => &[
            (50.0, 0.0),
            (100.0, 38.0),
            (82.0, 100.0),
            (18.0, 100.0),
            (0.0, 38.0),
        ],
        // DrawingML's hexagon is pointed left/right, not top/bottom.
        "hexagon" => &[
            (25.0, 0.0),
            (75.0, 0.0),
            (100.0, 50.0),
            (75.0, 100.0),
            (25.0, 100.0),
            (0.0, 50.0),
        ],
        "octagon" => &[
            (30.0, 0.0),
            (70.0, 0.0),
            (100.0, 30.0),
            (100.0, 70.0),
            (70.0, 100.0),
            (30.0, 100.0),
            (0.0, 70.0),
            (0.0, 30.0),
        ],
        "star4" => &[
            (50.0, 0.0),
            (62.0, 38.0),
            (100.0, 50.0),
            (62.0, 62.0),
            (50.0, 100.0),
            (38.0, 62.0),
            (0.0, 50.0),
            (38.0, 38.0),
        ],
        "star5" => &[
            (50.0, 0.0),
            (61.0, 35.0),
            (98.0, 35.0),
            (68.0, 57.0),
            (79.0, 91.0),
            (50.0, 70.0),
            (21.0, 91.0),
            (32.0, 57.0),
            (2.0, 35.0),
            (39.0, 35.0),
        ],
        "star6" => &[
            (50.0, 0.0),
            (66.0, 25.0),
            (100.0, 25.0),
            (83.0, 50.0),
            (100.0, 75.0),
            (66.0, 75.0),
            (50.0, 100.0),
            (34.0, 75.0),
            (0.0, 75.0),
            (17.0, 50.0),
            (0.0, 25.0),
            (34.0, 25.0),
        ],
        "rightArrow" => &[
            (0.0, 30.0),
            (60.0, 30.0),
            (60.0, 0.0),
            (100.0, 50.0),
            (60.0, 100.0),
            (60.0, 70.0),
            (0.0, 70.0),
        ],
        "leftArrow" => &[
            (40.0, 0.0),
            (40.0, 30.0),
            (100.0, 30.0),
            (100.0, 70.0),
            (40.0, 70.0),
            (40.0, 100.0),
            (0.0, 50.0),
        ],
        "upArrow" => &[
            (50.0, 0.0),
            (100.0, 40.0),
            (70.0, 40.0),
            (70.0, 100.0),
            (30.0, 100.0),
            (30.0, 40.0),
            (0.0, 40.0),
        ],
        "downArrow" => &[
            (30.0, 0.0),
            (70.0, 0.0),
            (70.0, 60.0),
            (100.0, 60.0),
            (50.0, 100.0),
            (0.0, 60.0),
            (30.0, 60.0),
        ],
        "chevron" => &[
            (0.0, 0.0),
            (75.0, 0.0),
            (100.0, 50.0),
            (75.0, 100.0),
            (0.0, 100.0),
            (25.0, 50.0),
        ],
        "homePlate" => &[
            (0.0, 0.0),
            (75.0, 0.0),
            (100.0, 50.0),
            (75.0, 100.0),
            (0.0, 100.0),
        ],
        "plus" => &[
            (35.0, 0.0),
            (65.0, 0.0),
            (65.0, 35.0),
            (100.0, 35.0),
            (100.0, 65.0),
            (65.0, 65.0),
            (65.0, 100.0),
            (35.0, 100.0),
            (35.0, 65.0),
            (0.0, 65.0),
            (0.0, 35.0),
            (35.0, 35.0),
        ],
        // A plaque's corners are cut by *concave* arcs. A polygon cannot curve,
        // and a concave notch reads as damage, so the corners are chamfered
        // instead: the same silhouette family, wrong curvature.
        "plaque" => &[
            (16.0, 0.0),
            (84.0, 0.0),
            (100.0, 16.0),
            (100.0, 84.0),
            (84.0, 100.0),
            (16.0, 100.0),
            (0.0, 84.0),
            (0.0, 16.0),
        ],
        _ => return None,
    })
}

/// `a:avLst/a:gd name="adj" fmla="val 16667"` → percent of the shorter side.
/// 16667 (thousandths of a percent) is the preset default when no adjust is
/// written.
fn round_rect_adj(geom: roxmltree::Node<'_, '_>) -> f64 {
    let mut pct = 16.667;
    if let Some(av) = child_elem(geom, "avLst") {
        for gd in elems(av).filter(|n| n.tag_name().name() == "gd") {
            // roundRect has a single handle; producers name it `adj` and (rarely)
            // `adj1`.
            if !matches!(xml::attr_local(gd, "name"), Some("adj") | Some("adj1") | None) {
                continue;
            }
            if let Some(v) = xml::attr_local(gd, "fmla")
                .and_then(|f| f.trim().strip_prefix("val ").map(|s| s.trim().to_string()))
                .and_then(|s| s.parse::<f64>().ok())
            {
                if v.is_finite() {
                    pct = v / 1000.0;
                }
            }
        }
    }
    // A radius over half the short side just means "fully rounded"; a negative
    // one is nonsense from a hostile part.
    pct.clamp(0.0, 50.0)
}

/// The corner radius in px for a box, for callers that know its size. CSS
/// percentage radii resolve per axis, so on a non-square box the percentage
/// spelling skews the corner; this is the un-skewed value.
pub fn round_radius_px(r_pct: f64, cx: f64, cy: f64) -> f64 {
    if !r_pct.is_finite() || !cx.is_finite() || !cy.is_finite() {
        return 0.0;
    }
    (r_pct.clamp(0.0, 50.0) / 100.0) * cx.abs().min(cy.abs())
}

/// The CSS declarations that give an element the geometry's outline. Empty for
/// [`Geom::Rect`] and [`Geom::Fallback`], which need nothing.
pub fn geom_css(g: &Geom) -> String {
    match g {
        Geom::Rect | Geom::Fallback(_) => String::new(),
        // Percentages resolve per axis, so a wide box gets elliptical corners
        // rather than circular ones. `round_radius_px` is the fix for callers
        // that have the box size.
        Geom::RoundRect { r_pct } => match fmt_pct(*r_pct as f32) {
            Some(v) => format!("border-radius:{};", v),
            None => String::new(),
        },
        Geom::Ellipse => "border-radius:50%;".to_string(),
        Geom::Poly(pts) => {
            if pts.len() < 3 {
                return String::new();
            }
            let mut parts = Vec::with_capacity(pts.len());
            for (x, y) in pts {
                // One unformattable vertex drops the whole clip-path: a partial
                // polygon would clip the element to nonsense.
                let (Some(px), Some(py)) = (fmt_pct(*x as f32), fmt_pct(*y as f32)) else {
                    return String::new();
                };
                parts.push(format!("{} {}", px, py));
            }
            format!("clip-path:polygon({});", parts.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

    fn doc_root(body: &str) -> roxmltree::Document<'_> {
        xml::parse(body).expect("fixture parses")
    }

    fn xf(body: &str) -> Option<Xf> {
        let src = format!("<a:spPr {}>{}</a:spPr>", NS, body);
        let doc = xml::parse(&src).expect("fixture parses");
        parse_xfrm(doc.root_element())
    }

    fn group(body: &str) -> Option<GroupXf> {
        let src = format!("<a:grpSpPr {}>{}</a:grpSpPr>", NS, body);
        let doc = xml::parse(&src).expect("fixture parses");
        parse_group_xfrm(doc.root_element())
    }

    fn geom(body: &str) -> Geom {
        let src = format!("<a:spPr {}>{}</a:spPr>", NS, body);
        let doc = xml::parse(&src).expect("fixture parses");
        parse_geom(doc.root_element())
    }

    /// px transform, for hand-written expectations.
    fn px_xf(x: f64, y: f64, cx: f64, cy: f64) -> Xf {
        Xf {
            x,
            y,
            cx,
            cy,
            rot: 0.0,
            flip_h: false,
            flip_v: false,
        }
    }

    fn emu(px: f64) -> i64 {
        (px * EMU_PER_PX) as i64
    }

    #[test]
    fn emu_px_uses_96dpi() {
        assert_eq!(emu_px(914400), 96.0);
        assert_eq!(emu_px(9525), 1.0);
        assert_eq!(emu_px(0), 0.0);
        assert_eq!(emu_px(-9525), -1.0);
    }

    #[test]
    fn parses_off_ext_rot_and_flips() {
        let x = xf(r#"<a:xfrm rot="2700000" flipH="1"><a:off x="914400" y="457200"/><a:ext cx="1828800" cy="914400"/></a:xfrm>"#)
            .expect("xfrm parses");
        assert_eq!((x.x, x.y), (96.0, 48.0));
        assert_eq!((x.cx, x.cy), (192.0, 96.0));
        assert_eq!(x.rot, 45.0); // 2700000 / 60000
        assert!(x.flip_h && !x.flip_v);
        // Negative rotation is legal.
        let x = xf(r#"<a:xfrm rot="-5400000" flipV="true"><a:off x="0" y="0"/><a:ext cx="9525" cy="9525"/></a:xfrm>"#)
            .unwrap();
        assert_eq!(x.rot, -90.0);
        assert!(x.flip_v);
        // A missing `a:off` places the box at the origin.
        let x = xf(r#"<a:xfrm><a:ext cx="9525" cy="19050"/></a:xfrm>"#).unwrap();
        assert_eq!((x.x, x.y, x.cx, x.cy), (0.0, 0.0, 1.0, 2.0));
    }

    #[test]
    fn malformed_xfrm_yields_none_without_panicking() {
        // No ext: nothing to place.
        assert!(xf("<a:xfrm/>").is_none());
        assert!(xf(r#"<a:xfrm><a:off x="0" y="0"/></a:xfrm>"#).is_none());
        // Unparseable extents.
        assert!(xf(r#"<a:xfrm><a:ext cx="café" cy="naïve"/></a:xfrm>"#).is_none());
        assert!(xf(r#"<a:xfrm><a:ext cx="99999999999999999999" cy="0"/></a:xfrm>"#).is_none());
        assert!(xf(r#"<a:xfrm><a:ext/></a:xfrm>"#).is_none());
        // No xfrm at all.
        assert!(xf(r#"<a:prstGeom prst="rect"/>"#).is_none());
        assert!(xf("").is_none());
        // Garbage rot/flip attributes degrade instead of failing.
        let x = xf(r#"<a:xfrm rot="Widget" flipH="maybe"><a:ext cx="9525" cy="9525"/></a:xfrm>"#)
            .unwrap();
        assert_eq!(x.rot, 0.0);
        assert!(!x.flip_h);
    }

    #[test]
    fn group_xfrm_defaults_the_child_space_to_the_group_box() {
        let g = group(r#"<a:xfrm><a:off x="9525" y="19050"/><a:ext cx="95250" cy="47625"/></a:xfrm>"#)
            .unwrap();
        // No chOff/chExt: the mapping is the identity, so a child composes to
        // exactly its own coordinates.
        assert_eq!(g.ch_off, (1.0, 2.0));
        assert_eq!(g.ch_ext, (10.0, 5.0));
        let child = px_xf(3.0, 4.0, 2.0, 2.0);
        assert_eq!(
            Xf::compose(&g.xf, g.ch_off, g.ch_ext, &child),
            px_xf(3.0, 4.0, 2.0, 2.0)
        );
    }

    #[test]
    fn compose_translates_scales_and_translates() {
        // group: off (96,48) px, ext (192,96) px, child space (96,48) px at origin
        //   → scale = 192/96 = 2 on x, 96/48 = 2 on y
        // child: off (48,24) px, ext (48,24) px
        //   x  = 96 + (48 - 0)*2 = 192
        //   y  = 48 + (24 - 0)*2 = 96
        //   cx = 48*2 = 96,  cy = 24*2 = 48
        let g = group(&format!(
            r#"<a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/><a:chOff x="0" y="0"/><a:chExt cx="{}" cy="{}"/></a:xfrm>"#,
            emu(96.0),
            emu(48.0),
            emu(192.0),
            emu(96.0),
            emu(96.0),
            emu(48.0)
        ))
        .unwrap();
        let child = px_xf(48.0, 24.0, 48.0, 24.0);
        let out = Xf::compose(&g.xf, g.ch_off, g.ch_ext, &child);
        assert_eq!(out, px_xf(192.0, 96.0, 96.0, 48.0));

        // A non-zero chOff is subtracted before scaling: a child sitting exactly
        // at the child-space origin lands on the group's own origin.
        let g2 = group(&format!(
            r#"<a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/><a:chOff x="{}" y="{}"/><a:chExt cx="{}" cy="{}"/></a:xfrm>"#,
            emu(96.0),
            emu(48.0),
            emu(192.0),
            emu(96.0),
            emu(48.0),
            emu(24.0),
            emu(96.0),
            emu(48.0)
        ))
        .unwrap();
        let out = Xf::compose(&g2.xf, g2.ch_off, g2.ch_ext, &child);
        assert_eq!(out, px_xf(96.0, 48.0, 96.0, 48.0));
    }

    #[test]
    fn compose_accumulates_rotation_and_flips() {
        let g = GroupXf {
            xf: Xf {
                rot: 30.0,
                flip_h: true,
                ..px_xf(0.0, 0.0, 100.0, 100.0)
            },
            ch_off: (0.0, 0.0),
            ch_ext: (100.0, 100.0),
        };
        let child = Xf {
            rot: 15.0,
            flip_h: true,
            flip_v: true,
            ..px_xf(0.0, 0.0, 10.0, 10.0)
        };
        let out = Xf::compose(&g.xf, g.ch_off, g.ch_ext, &child);
        assert_eq!(out.rot, 45.0);
        // Two horizontal flips cancel; the vertical one survives.
        assert!(!out.flip_h);
        assert!(out.flip_v);
    }

    #[test]
    fn nested_groups_compose_two_levels() {
        // outer: off (0,0), ext (400,200), child space (200,100) → scale 2
        // inner: off (50,25), ext (100,50), child space chOff (10,5) chExt (50,25)
        //   mapped by outer: x = 0 + 50*2 = 100, y = 0 + 25*2 = 50
        //                    cx = 100*2 = 200,   cy = 50*2 = 100
        //   inner scale for its children: 200/50 = 4, 100/25 = 4
        // leaf: off (20,10), ext (30,15)
        //   x  = 100 + (20-10)*4 = 140
        //   y  =  50 + (10-5)*4  =  70
        //   cx = 30*4 = 120,  cy = 15*4 = 60
        let outer = GroupXf {
            xf: px_xf(0.0, 0.0, 400.0, 200.0),
            ch_off: (0.0, 0.0),
            ch_ext: (200.0, 100.0),
        };
        let inner = GroupXf {
            xf: px_xf(50.0, 25.0, 100.0, 50.0),
            ch_off: (10.0, 5.0),
            ch_ext: (50.0, 25.0),
        };
        let leaf = px_xf(20.0, 10.0, 30.0, 15.0);

        // Step by step, then through the helper: both must agree.
        let inner_mapped = Xf::compose(&outer.xf, outer.ch_off, outer.ch_ext, &inner.xf);
        assert_eq!(inner_mapped, px_xf(100.0, 50.0, 200.0, 100.0));
        let stepwise = Xf::compose(&inner_mapped, inner.ch_off, inner.ch_ext, &leaf);
        assert_eq!(stepwise, px_xf(140.0, 70.0, 120.0, 60.0));
        assert_eq!(Xf::compose_nested(&[outer, inner], &leaf), stepwise);

        // An empty chain leaves the child alone.
        assert_eq!(Xf::compose_nested(&[], &leaf), leaf);
    }

    #[test]
    fn zero_child_extent_never_produces_nan() {
        let g = GroupXf {
            xf: px_xf(10.0, 20.0, 100.0, 50.0),
            ch_off: (0.0, 0.0),
            ch_ext: (0.0, 0.0),
        };
        let child = px_xf(5.0, 5.0, 30.0, 30.0);
        let out = Xf::compose(&g.xf, g.ch_off, g.ch_ext, &child);
        for v in [out.x, out.y, out.cx, out.cy, out.rot] {
            assert!(v.is_finite(), "{:?} holds a non-finite coordinate", out);
        }
        // Scale falls back to 1, so the child is merely translated.
        assert_eq!(out, px_xf(15.0, 25.0, 30.0, 30.0));
        // And the CSS carries no NaN.
        let css = out.css();
        assert!(!css.contains("NaN"), "{}", css);
        assert!(css.starts_with("left:15px;top:25px;width:30px;height:30px;"), "{}", css);

        // Same for a chExt that is only zero on one axis, and for a poisoned
        // group extent.
        let half = GroupXf {
            ch_ext: (50.0, 0.0),
            ..g
        };
        let out = Xf::compose(&half.xf, half.ch_off, half.ch_ext, &child);
        assert!(out.cx == 60.0 && out.cy == 30.0, "{:?}", out);
        let poisoned = GroupXf {
            xf: Xf {
                cx: f64::NAN,
                ..g.xf
            },
            ch_ext: (50.0, 25.0),
            ..g
        };
        let out = Xf::compose(&poisoned.xf, poisoned.ch_off, poisoned.ch_ext, &child);
        assert!(out.cx.is_finite() && out.cy.is_finite(), "{:?}", out);
    }

    #[test]
    fn group_chain_is_depth_bounded() {
        // A chain far deeper than the limit must still return finite numbers
        // promptly; only the outermost MAX_GROUP_DEPTH levels are applied.
        let g = GroupXf {
            xf: px_xf(1.0, 1.0, 20.0, 20.0),
            ch_off: (0.0, 0.0),
            ch_ext: (10.0, 10.0), // doubles at every level
        };
        let chain = vec![g; MAX_GROUP_DEPTH + 500];
        let out = Xf::compose_nested(&chain, &px_xf(0.0, 0.0, 1.0, 1.0));
        assert!(out.cx.is_finite() && out.cy.is_finite());
        // Each level doubles (ext 20 over chExt 10); 16 levels applied to a 1px
        // child gives 2^16, and the 500 extra levels are dropped.
        assert_eq!(out.cx, 2f64.powi(16));
        assert_eq!(Xf::compose_nested(&chain[..1], &px_xf(0.0, 0.0, 1.0, 1.0)).cx, 2.0);
    }

    #[test]
    fn xf_css_emits_position_size_and_transform() {
        let x = Xf {
            rot: 45.0,
            flip_h: true,
            ..px_xf(10.0, 20.5, 100.0, 50.0)
        };
        let css = x.css();
        assert!(css.contains("left:10px;top:20.5px;width:100px;height:50px;"), "{}", css);
        assert!(css.contains("transform:rotate(45.00deg) scale(-1, 1);"), "{}", css);
        // No rotation and no flips: no transform declaration at all.
        assert_eq!(
            px_xf(0.0, 0.0, 1.0, 1.0).css(),
            "left:0px;top:0px;width:1px;height:1px;"
        );
    }

    #[test]
    fn tier1_presets_are_pure_css() {
        assert_eq!(geom(r#"<a:prstGeom prst="rect"/>"#), Geom::Rect);
        assert_eq!(geom_css(&Geom::Rect), "");
        assert_eq!(geom(r#"<a:prstGeom prst="ellipse"/>"#), Geom::Ellipse);
        assert_eq!(geom(r#"<a:prstGeom prst="circle"/>"#), Geom::Ellipse);
        assert_eq!(geom_css(&Geom::Ellipse), "border-radius:50%;");
        assert_eq!(geom(r#"<a:prstGeom prst="line"/>"#), Geom::Rect);
        assert_eq!(geom(r#"<a:prstGeom prst="bevel"/>"#), Geom::Rect);
    }

    #[test]
    fn round_rect_honours_its_adjust_handle() {
        // 25000 thousandths of a percent = 25% of the shorter side.
        let g = geom(
            r#"<a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="val 25000"/></a:avLst></a:prstGeom>"#,
        );
        assert_eq!(g, Geom::RoundRect { r_pct: 25.0 });
        assert_eq!(geom_css(&g), "border-radius:25%;");
        // No avLst: the preset default of 16667 (16.667%).
        let g = geom(r#"<a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom>"#);
        let Geom::RoundRect { r_pct } = g else {
            panic!("expected a roundRect")
        };
        assert!((r_pct - 16.667).abs() < 1e-6);
        assert_eq!(geom_css(&g), "border-radius:16.67%;");
        // Hostile adjusts clamp instead of producing a negative radius.
        for (fmla, want) in [("val -9000", 0.0), ("val 900000", 50.0), ("café", 16.667)] {
            let g = geom(&format!(
                r#"<a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="{}"/></a:avLst></a:prstGeom>"#,
                fmla
            ));
            let Geom::RoundRect { r_pct } = g else {
                panic!("expected a roundRect")
            };
            assert!((r_pct - want).abs() < 1e-6, "{} → {}", fmla, r_pct);
        }
        // The px helper un-skews the radius on a non-square box.
        assert_eq!(round_radius_px(25.0, 400.0, 100.0), 25.0);
        assert_eq!(round_radius_px(f64::NAN, 400.0, 100.0), 0.0);
    }

    #[test]
    fn tier2_presets_clip_with_percentage_polygons() {
        let g = geom(r#"<a:prstGeom prst="triangle"><a:avLst/></a:prstGeom>"#);
        assert_eq!(
            g,
            Geom::Poly(vec![(50.0, 0.0), (100.0, 100.0), (0.0, 100.0)])
        );
        assert_eq!(geom_css(&g), "clip-path:polygon(50% 0%, 100% 100%, 0% 100%);");
        // Every shipped tier-2 preset must produce a usable clip-path.
        for prst in [
            "triangle",
            "rtTriangle",
            "diamond",
            "parallelogram",
            "trapezoid",
            "pentagon",
            "hexagon",
            "octagon",
            "star4",
            "star5",
            "star6",
            "rightArrow",
            "leftArrow",
            "upArrow",
            "downArrow",
            "chevron",
            "homePlate",
            "plus",
            "plaque",
        ] {
            let g = geom(&format!(r#"<a:prstGeom prst="{}"/>"#, prst));
            let Geom::Poly(pts) = &g else {
                panic!("{} should be a polygon, got {:?}", prst, g)
            };
            assert!(pts.len() >= 3, "{} has too few vertices", prst);
            for (x, y) in pts {
                assert!(
                    (0.0..=100.0).contains(x) && (0.0..=100.0).contains(y),
                    "{} vertex ({}, {}) is outside the box",
                    prst,
                    x,
                    y
                );
            }
            let css = geom_css(&g);
            assert!(css.starts_with("clip-path:polygon("), "{} → {}", prst, css);
            assert!(css.ends_with(");"), "{} → {}", prst, css);
        }
        // An adjust handle on a tier-2 preset is ignored, not honoured.
        let adjusted = geom(
            r#"<a:prstGeom prst="chevron"><a:avLst><a:gd name="adj" fmla="val 40000"/></a:avLst></a:prstGeom>"#,
        );
        assert_eq!(adjusted, geom(r#"<a:prstGeom prst="chevron"/>"#));
    }

    #[test]
    fn unknown_presets_and_custom_geometry_fall_back() {
        assert_eq!(
            geom(r#"<a:prstGeom prst="Widget"/>"#),
            Geom::Fallback("Widget".to_string())
        );
        assert_eq!(
            geom(r#"<a:prstGeom prst="wedgeRoundRectCallout"/>"#),
            Geom::Fallback("wedgeRoundRectCallout".to_string())
        );
        assert_eq!(
            geom(r#"<a:custGeom><a:pathLst><a:path w="100" h="100"/></a:pathLst></a:custGeom>"#),
            Geom::Fallback("custGeom".to_string())
        );
        // A fallback draws as a plain box, so it contributes no CSS.
        assert_eq!(geom_css(&Geom::Fallback("Widget".to_string())), "");
        // No geometry element at all (a picture, a text box) is a rectangle.
        assert_eq!(geom(""), Geom::Rect);
        assert_eq!(geom(r#"<a:prstGeom/>"#), Geom::Rect);
    }

    #[test]
    fn geom_css_drops_a_polygon_with_an_unusable_vertex() {
        // A partial clip-path would clip the element to nonsense, so one bad
        // vertex drops the whole declaration.
        assert_eq!(geom_css(&Geom::Poly(vec![(0.0, 0.0), (f64::NAN, 50.0), (100.0, 100.0)])), "");
        assert_eq!(geom_css(&Geom::Poly(vec![(0.0, 0.0), (100.0, 100.0)])), "");
        assert_eq!(geom_css(&Geom::RoundRect { r_pct: f64::NAN }), "");
    }

    #[test]
    fn geometry_and_transform_can_be_read_off_a_real_sp_pr() {
        // The shape of the thing callers actually hold: a `p:spPr` with the
        // transform and the geometry side by side.
        let src = format!(
            r#"<p:spPr xmlns:p="p" {}>
                 <a:xfrm rot="900000"><a:off x="914400" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm>
                 <a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="val 10000"/></a:avLst></a:prstGeom>
                 <a:solidFill><a:srgbClr val="4472C4"/></a:solidFill>
               </p:spPr>"#,
            NS
        );
        let doc = doc_root(&src);
        let x = parse_xfrm(doc.root_element()).expect("xfrm parses");
        assert_eq!((x.x, x.y, x.cx, x.cy), (96.0, 0.0, 96.0, 48.0));
        assert_eq!(x.rot, 15.0);
        assert_eq!(
            parse_geom(doc.root_element()),
            Geom::RoundRect { r_pct: 10.0 }
        );
    }
}
