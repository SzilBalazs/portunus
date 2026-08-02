// Geometry for the virtual selection engine: caret hit-testing and
// range → highlight-rect conversion. All output coordinates are
// content-local to the selectable root (scroll- and zoom-independent:
// the overlay lives inside the scroll/transform container).

export interface CaretPos {
  node: Text;
  offset: number;
}

/** Content-local rectangle inside a selectable root, in untransformed px. */
export interface SelRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

function firstTextIn(node: Node): Text | null {
  if (node.nodeType === Node.TEXT_NODE) return node as Text;
  const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT);
  return walker.nextNode() as Text | null;
}

function lastTextIn(node: Node): Text | null {
  if (node.nodeType === Node.TEXT_NODE) return node as Text;
  const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT);
  let last: Text | null = null;
  for (let t = walker.nextNode(); t; t = walker.nextNode()) last = t as Text;
  return last;
}

/** Resolve an element-boundary hit to the nearest text position in document
 *  order. `backward` picks the tail of the preceding content (used when the
 *  point is past the end of a line/block). */
function resolveToText(node: Node, offset: number, backward: boolean): CaretPos | null {
  if (node.nodeType === Node.TEXT_NODE) {
    return { node: node as Text, offset };
  }
  const children = node.childNodes;
  if (children.length === 0) {
    const t = backward ? lastTextIn(node) : firstTextIn(node);
    return t ? { node: t, offset: backward ? t.data.length : 0 } : null;
  }
  // Search outwards from the boundary child for the nearest text node.
  const idx = Math.min(offset, children.length - 1);
  if (backward) {
    for (let i = Math.min(offset, children.length) - 1; i >= 0; i--) {
      const t = lastTextIn(children[i]);
      if (t) return { node: t, offset: t.data.length };
    }
    const t = firstTextIn(node);
    return t ? { node: t, offset: 0 } : null;
  }
  for (let i = idx; i < children.length; i++) {
    const t = firstTextIn(children[i]);
    if (t) return { node: t, offset: 0 };
  }
  const t = lastTextIn(node);
  return t ? { node: t, offset: t.data.length } : null;
}

/** The character offset in a text node nearest viewport x on the line at y.
 *  Scans each glyph's rect (scroll- and transform-safe, unlike
 *  caretRangeFromPoint) and splits at the glyph midpoint. */
export function offsetAtX(node: Text, x: number, y: number): number {
  const r = document.createRange();
  let best = 0;
  let bestDist = Infinity;
  for (let i = 0; i < node.data.length; i++) {
    r.setStart(node, i);
    r.setEnd(node, i + 1);
    for (const rect of r.getClientRects()) {
      if (rect.width <= 0 && rect.height <= 0) continue;
      // Prefer glyphs on the clicked line; fall back to nearest by center.
      const onLine = y >= rect.top - 1 && y <= rect.bottom + 1;
      const mid = rect.left + rect.width / 2;
      const dist = Math.abs(x - mid) + (onLine ? 0 : 1e6 + Math.abs(y - (rect.top + rect.height / 2)));
      if (dist < bestDist) {
        bestDist = dist;
        best = x > mid ? i + 1 : i;
      }
    }
  }
  return best;
}

/** Manual, scroll-safe hit-test: find the text node under (x, y) within `root`
 *  and the caret offset in it. Ranges' getClientRects report true post-scroll,
 *  post-transform viewport positions, so this works where WebKitGTK's
 *  caretRangeFromPoint fails to descend into scrolled overflow containers. */
function hitTestText(container: Element, x: number, y: number, root: HTMLElement): CaretPos | null {
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT);
  const r = document.createRange();
  let onLine: Text | null = null;
  let nearest: Text | null = null;
  let nearestDist = Infinity;
  for (let t = walker.nextNode() as Text | null; t; t = walker.nextNode() as Text | null) {
    if (t.data.length === 0 || !root.contains(t)) continue;
    r.selectNodeContents(t);
    for (const rect of r.getClientRects()) {
      if (rect.width <= 0 || rect.height <= 0) continue;
      if (y >= rect.top - 1 && y <= rect.bottom + 1 && x >= rect.left - 1 && x <= rect.right + 1) {
        onLine = t;
        break;
      }
      const cx = rect.left + rect.width / 2;
      const cy = rect.top + rect.height / 2;
      const d = Math.hypot(x - cx, y - cy);
      if (d < nearestDist) { nearestDist = d; nearest = t; }
    }
    if (onLine) break;
  }
  const node = onLine ?? nearest;
  return node ? { node, offset: offsetAtX(node, x, y) } : null;
}

/** Caret position at a viewport point, snapped into `root`. Points outside the
 *  root are clamped to its border box first so drags past the edges keep
 *  extending the selection. Returns null when the root holds no text. */
export function caretFromPoint(x: number, y: number, root: HTMLElement): CaretPos | null {
  const bounds = root.getBoundingClientRect();
  const cx = Math.min(Math.max(x, bounds.left + 1), bounds.right - 1);
  const cy = Math.min(Math.max(y, bounds.top + 1), bounds.bottom - 1);

  // Manual hit-test starting from the element actually painted at the point
  // (elementFromPoint respects scroll/transform). This is the reliable path;
  // caretRangeFromPoint is only a fallback because WebKitGTK mis-handles it
  // inside scrolled overflow containers (late lines return null).
  const el = document.elementFromPoint(cx, cy);
  const container = el && root.contains(el) ? el : root;
  const hit = hitTestText(container, cx, cy, root);
  if (hit) return hit;

  // Fallback: the native API (covers points that landed on non-text chrome).
  const doc = document as Document & {
    caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null;
  };
  const backward = y > bounds.bottom || (y >= bounds.top && x > bounds.right);
  let node: Node | null = null;
  let offset = 0;
  if (typeof doc.caretRangeFromPoint === "function") {
    const r = doc.caretRangeFromPoint(cx, cy);
    if (r) { node = r.startContainer; offset = r.startOffset; }
  } else if (typeof doc.caretPositionFromPoint === "function") {
    const p = doc.caretPositionFromPoint(cx, cy);
    if (p) { node = p.offsetNode; offset = p.offset; }
  }
  if (!node || !root.contains(node)) return null;
  return resolveToText(node, offset, backward);
}

/** Text nodes intersecting a range, in document order. Shared by rect
 *  computation and text extraction so both agree on coverage. */
export function textNodesInRange(range: Range): Text[] {
  const rootNode = range.commonAncestorContainer;
  const walker = document.createTreeWalker(
    rootNode.nodeType === Node.TEXT_NODE ? rootNode.parentNode ?? rootNode : rootNode,
    NodeFilter.SHOW_TEXT,
  );
  const out: Text[] = [];
  for (let t = walker.nextNode(); t; t = walker.nextNode()) {
    if (range.intersectsNode(t)) out.push(t as Text);
  }
  return out;
}

/** The intersection of `range` with a single text node, as [start, end) offsets. */
export function rangeOffsetsInNode(range: Range, node: Text): [number, number] {
  const start = node === range.startContainer ? range.startOffset : 0;
  const end = node === range.endContainer ? range.endOffset : node.data.length;
  return [start, end];
}

/** Merge per-fragment rects into one bar per visual line. Two rects share a
 *  line when their vertical overlap covers most of the smaller one. */
function mergeLineRects(rects: SelRect[]): SelRect[] {
  if (rects.length <= 1) return rects;
  const sorted = [...rects].sort((a, b) => a.y - b.y || a.x - b.x);
  const lines: SelRect[] = [];
  for (const r of sorted) {
    const line = lines[lines.length - 1];
    if (line) {
      const overlap = Math.min(line.y + line.h, r.y + r.h) - Math.max(line.y, r.y);
      if (overlap > 0.5 * Math.min(line.h, r.h)) {
        const x = Math.min(line.x, r.x);
        const y = Math.min(line.y, r.y);
        line.w = Math.max(line.x + line.w, r.x + r.w) - x;
        line.h = Math.max(line.y + line.h, r.y + r.h) - y;
        line.x = x;
        line.y = y;
        continue;
      }
    }
    lines.push({ ...r });
  }
  return lines;
}

/** CSS size of a `.sel-probe` box, kept in sync with the App.css rule. */
export const PROBE_PX = 100;

/**
 * Nearest ancestor that clips `el` — anything with a non-`visible` overflow, not
 * only `auto`/`scroll`.
 *
 * That distinction is the whole point: the PDF reader zooms and pans with a
 * transform and lets `.pdf-ql` clip (`overflow: hidden`), so it never scrolls and
 * a scroller-only walk finds nothing. Treating the page itself as the visible box
 * then puts the selection popover somewhere off-screen whenever the page is zoomed
 * past fit. Scrolling still needs the narrower `auto|scroll` test — a clipping box
 * has no scrollTop to move — so that one stays where it is used, in the controller.
 */
export function clipAncestor(el: HTMLElement): HTMLElement | null {
  for (let n: HTMLElement | null = el; n; n = n.parentElement) {
    const s = getComputedStyle(n);
    if (s.overflowY !== "visible" || s.overflowX !== "visible") return n;
  }
  return null;
}

/** The on-screen part of a root, clipped by whatever encloses it. */
export function visibleRect(root: HTMLElement): DOMRect {
  const r = root.getBoundingClientRect();
  const clip = clipAncestor(root);
  if (!clip) return r;
  const v = clip.getBoundingClientRect();
  const left = Math.max(r.left, v.left);
  const top = Math.max(r.top, v.top);
  return new DOMRect(
    left,
    top,
    Math.max(0, Math.min(r.right, v.right) - left),
    Math.max(0, Math.min(r.bottom, v.bottom) - top),
  );
}

/** A positioning space: where its origin is painted, and how many painted pixels
 *  one authored CSS pixel inside it covers. */
export interface PaintFrame {
  x: number;
  y: number;
  sx: number;
  sy: number;
}

/**
 * Measure a positioning space from a `.sel-probe` — a hidden box the caller
 * renders at `left:0;top:0` inside it, at a known CSS size.
 *
 * Two independent things scale these spaces: `zoom` (the launcher's whole UI runs
 * under `documentElement.style.zoom`) and `transform` (the PDF Quicklook's page
 * scale). A rect converted with only one of them is out by a percentage of its
 * distance from the origin — right at the top of a preview, visibly wrong further
 * down. Deriving the factor from `offsetWidth` looks like it handles both, but
 * whether `offset*` divides `zoom` back out is an engine detail; a box laid out at
 * a known CSS size in the space itself needs no such assumption, and its painted
 * position is that space's origin (no assumption about the parent's padding
 * either). Returns null when the probe has not been laid out yet.
 */
export function probeFrame(probe: Element | null): PaintFrame | null {
  if (!probe) return null;
  const r = probe.getBoundingClientRect();
  if (!(r.width > 0) || !(r.height > 0)) return null;
  return { x: r.left, y: r.top, sx: r.width / PROBE_PX, sy: r.height / PROBE_PX };
}

/** Highlight rects for a range, in the coordinates of the space `frame` measures —
 *  the overlay the rects render inside. Because the offset is measured against
 *  that space's OWN painted origin, this is correct whether the overlay scrolls
 *  with the content or is pinned (WebKitGTK differs by element box), and stays
 *  correct across scroll as long as it is recomputed with a fresh frame. Walks
 *  text nodes rather than Range.getClientRects (which adds contained elements'
 *  boxes). */
export function rectsForRange(range: Range, frame: PaintFrame): SelRect[] {
  const out: SelRect[] = [];
  const sub = document.createRange();
  for (const node of textNodesInRange(range)) {
    const [start, end] = rangeOffsetsInNode(range, node);
    if (start >= end) continue;
    sub.setStart(node, start);
    sub.setEnd(node, end);
    for (const r of sub.getClientRects()) {
      if (r.width <= 0 || r.height <= 0) continue;
      out.push({
        x: (r.left - frame.x) / frame.sx,
        y: (r.top - frame.y) / frame.sy,
        w: r.width / frame.sx,
        h: r.height / frame.sy,
      });
    }
  }
  return mergeLineRects(out);
}

/** Viewport rect of a collapsed caret position (zero width). */
export function caretClientRect(pos: CaretPos): DOMRect | null {
  const r = document.createRange();
  r.setStart(pos.node, pos.offset);
  r.collapse(true);
  const rects = r.getClientRects();
  if (rects.length > 0) return rects[0];
  // Collapsed ranges at some boundaries report no rect; fall back to the
  // bounding rect of the character beside the caret.
  const len = pos.node.data.length;
  if (len === 0) return null;
  const probe = document.createRange();
  const at = Math.min(pos.offset, len - 1);
  probe.setStart(pos.node, at);
  probe.setEnd(pos.node, at + 1);
  const pr = probe.getBoundingClientRect();
  if (pr.height <= 0) return null;
  return new DOMRect(pos.offset >= len ? pr.right : pr.left, pr.top, 0, pr.height);
}

/** Rect of a collapsed caret position, in the same space as `rectsForRange`. */
export function caretRect(pos: CaretPos, frame: PaintFrame): SelRect | null {
  const rect = caretClientRect(pos);
  if (!rect) return null;
  return {
    x: (rect.left - frame.x) / frame.sx,
    y: (rect.top - frame.y) / frame.sy,
    w: 0,
    h: rect.height / frame.sy,
  };
}

/** All text nodes under a root, in document order. */
export function allTextNodes(root: HTMLElement): Text[] {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const out: Text[] = [];
  for (let t = walker.nextNode(); t; t = walker.nextNode()) out.push(t as Text);
  return out;
}
