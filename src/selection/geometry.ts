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

/** Reads the zoom factor a selectable root is rendered under (PDF Quicklook
 *  applies a CSS scale transform on an ancestor; DOM previews are 1). */
/** The effective CSS scale between `root`'s layout box and its painted box,
 *  derived empirically (getBoundingClientRect is post-transform; offset* are
 *  pre-transform). Captures the PDF Quicklook zoom — and any ancestor
 *  transform — without a hand-maintained attribute. `1` when untransformed. */
export function rootScale(root: HTMLElement): { x: number; y: number } {
  const rect = root.getBoundingClientRect();
  const x = root.offsetWidth > 0 ? rect.width / root.offsetWidth : 1;
  const y = root.offsetHeight > 0 ? rect.height / root.offsetHeight : 1;
  return {
    x: Number.isFinite(x) && x > 0 ? x : 1,
    y: Number.isFinite(y) && y > 0 ? y : 1,
  };
}

/** Highlight rects for a range, positioned relative to `originRect` — the live
 *  bounding rect of the overlay element the rects render inside. Because the
 *  offset is measured against the overlay's OWN painted position, this is
 *  correct whether the overlay scrolls with the content or is pinned (WebKitGTK
 *  differs by element box), and stays correct across scroll as long as it is
 *  recomputed with a fresh `originRect`. `scale` is the overlay/root
 *  painted-vs-layout ratio (PDF zoom, ancestor transforms). Walks text nodes
 *  rather than Range.getClientRects (which adds contained elements' boxes). */
export function rectsForRange(range: Range, originRect: DOMRect, scale: { x: number; y: number }): SelRect[] {
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
        x: (r.left - originRect.left) / scale.x,
        y: (r.top - originRect.top) / scale.y,
        w: r.width / scale.x,
        h: r.height / scale.y,
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

/** Rect of a collapsed caret position, relative to `originRect` (same overlay
 *  space as `rectsForRange`). */
export function caretRect(pos: CaretPos, originRect: DOMRect, scale: { x: number; y: number }): SelRect | null {
  const rect = caretClientRect(pos);
  if (!rect) return null;
  return {
    x: (rect.left - originRect.left) / scale.x,
    y: (rect.top - originRect.top) / scale.y,
    w: 0,
    h: rect.height / scale.y,
  };
}

/** All text nodes under a root, in document order. */
export function allTextNodes(root: HTMLElement): Text[] {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const out: Text[] = [];
  for (let t = walker.nextNode(); t; t = walker.nextNode()) out.push(t as Text);
  return out;
}
