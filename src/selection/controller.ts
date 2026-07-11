// Virtual selection controller: a module-level singleton (same idiom as the
// command store) that tracks mouse drags over `[data-selectable]` preview
// roots and exposes the current selection to React via useSyncExternalStore.
//
// The search input never loses focus: the card-level mousedown preventDefault
// stops the browser from moving focus or starting a native selection, and this
// controller re-implements selection with caret hit-testing + custom-rendered
// highlight rects (WebKit cannot paint a styled selection outside the focused
// element - its FrameSelection is unified with the input caret).

import { invoke } from "@tauri-apps/api/core";
import {
  CaretPos,
  allTextNodes,
  caretClientRect,
  caretFromPoint,
  offsetAtX,
} from "./geometry";
import { extractText, separatesLine } from "./extract";

const WORD_CHAR = /[\p{L}\p{N}_]/u;

/** True when caret `a` is at or before caret `b` in document order. */
function beforeOrEqual(a: CaretPos, b: CaretPos): boolean {
  if (a.node === b.node) return a.offset <= b.offset;
  const pos = a.node.compareDocumentPosition(b.node);
  return (pos & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
}

/** Keyboard-mode movement chords (modifier handling happens at dispatch). */
const MOVE_KEYS: Record<string, "left" | "right" | "up" | "down" | "home" | "end"> = {
  ArrowLeft: "left",
  ArrowRight: "right",
  ArrowUp: "up",
  ArrowDown: "down",
  Home: "home",
  End: "end",
};

function scrollableAncestor(el: HTMLElement): HTMLElement | null {
  for (let n: HTMLElement | null = el; n; n = n.parentElement) {
    const s = getComputedStyle(n);
    if (/(auto|scroll)/.test(s.overflowY + s.overflowX)) return n;
  }
  return null;
}

/** The on-screen part of a root that may be clipped by its scroll container. */
function visibleRect(root: HTMLElement): DOMRect {
  const r = root.getBoundingClientRect();
  const scroller = scrollableAncestor(root);
  if (!scroller) return r;
  const v = scroller.getBoundingClientRect();
  const left = Math.max(r.left, v.left);
  const top = Math.max(r.top, v.top);
  return new DOMRect(
    left,
    top,
    Math.max(0, Math.min(r.right, v.right) - left),
    Math.max(0, Math.min(r.bottom, v.bottom) - top),
  );
}

/** Word boundaries around an offset within one text node ([start, end), or
 *  the single adjacent character for punctuation hits). */
function wordBoundsAt(data: string, offset: number): [number, number] | null {
  if (data.length === 0) return null;
  let s = offset;
  let e = offset;
  const wordAfter = e < data.length && WORD_CHAR.test(data[e]);
  const wordBefore = s > 0 && WORD_CHAR.test(data[s - 1]);
  if (wordAfter || wordBefore) {
    while (s > 0 && WORD_CHAR.test(data[s - 1])) s--;
    while (e < data.length && WORD_CHAR.test(data[e])) e++;
    return [s, e];
  }
  if (e < data.length) return [e, e + 1];
  return [s - 1, s];
}

export interface SelectionSnapshot {
  /** Active selectable root (portal target for the overlay), null = no selection. */
  root: HTMLElement | null;
  /** The live selection range (rects are measured from it at render time, in
   *  SelectionLayer, relative to the overlay's own box — engine-agnostic). */
  range: Range | null;
  /** The caret/focus end (for the keyboard cursor and popover anchor). */
  focus: CaretPos | null;
  /** Mid-drag (popover stays hidden until mouseup). */
  dragging: boolean;
  /** Keyboard select mode active. */
  keyboard: boolean;
}

const EMPTY: SelectionSnapshot = {
  root: null,
  range: null,
  focus: null,
  dragging: false,
  keyboard: false,
};

const DRAG_THRESHOLD = 3;

class SelectionController {
  private listeners = new Set<() => void>();
  private snapshot: SelectionSnapshot = EMPTY;

  private root: HTMLElement | null = null;
  private anchor: CaretPos | null = null;
  private focus: CaretPos | null = null;
  private keyboard = false;

  // Drag tracking.
  private pending: { root: HTMLElement; x: number; y: number } | null = null;
  private dragging = false;
  private suppressClick = false;
  // Granularity of the active drag: "char" = normal drag; "word"/"line" = a
  // drag started from a double/triple-click, which extends by whole words/lines
  // anchored on the initially hit word/line (`dragAnchorBounds`).
  private dragGranularity: "char" | "word" | "line" = "char";
  private dragAnchorBounds: [CaretPos, CaretPos] | null = null;

  // Invalidation of a live selection when the preview re-renders under it.
  private observer: MutationObserver | null = null;

  constructor() {
    document.addEventListener("mousedown", this.onMouseDown, true);
    document.addEventListener("click", this.onClickCapture, true);
  }

  subscribe = (cb: () => void) => {
    this.listeners.add(cb);
    return () => void this.listeners.delete(cb);
  };

  getSnapshot = (): SelectionSnapshot => this.snapshot;

  hasSelection(): boolean {
    return this.range() !== null;
  }

  isKeyboardMode(): boolean {
    return this.keyboard;
  }

  getText(): string {
    const r = this.range();
    return r ? extractText(r) : "";
  }

  /** Copy the selection. wl-copy backend first (survives the launcher hiding
   *  on Wayland); webview clipboard as fallback. */
  copy(): void {
    const text = this.getText();
    if (!text) return;
    invoke("copy_text", { text }).catch(() => {
      navigator.clipboard.writeText(text).catch(() => {});
    });
  }

  clear(): void {
    this.cancelDrag();
    if (!this.root && !this.keyboard) return;
    this.root = null;
    this.anchor = null;
    this.focus = null;
    this.exitKeyboard();
    this.disconnectObserver();
    this.emit();
  }

  // ── keyboard select mode ────────────────────────────────────────────────────

  /** Enter caret mode on a selectable root (Ctrl+S). The caret starts at the
   *  current selection's focus, else at the first visible text position. */
  enterKeyboardMode(root: HTMLElement | null): boolean {
    if (!root) return false;
    let caret = this.root === root && this.focus?.node.isConnected ? this.focus : null;
    if (!caret) {
      const visible = visibleRect(root);
      caret =
        caretFromPoint(visible.left + 4, visible.top + 4, root) ??
        caretFromPoint(visible.left + visible.width / 2, visible.top + visible.height / 2, root);
    }
    if (!caret) return false;
    this.cancelDrag();
    this.root = root;
    this.focus = caret;
    this.anchor = null;
    this.keyboard = true;
    this.goalX = null;
    this.observeRoot(root);
    window.addEventListener("keydown", this.onKeyboardKey, true);
    this.emit();
    return true;
  }

  /** Linear character offset of a caret within its root's text stream. */
  private linearOffset(pos: CaretPos): number | null {
    let acc = 0;
    for (const n of allTextNodes(this.root!)) {
      if (n === pos.node) return acc + pos.offset;
      acc += n.data.length;
    }
    return null;
  }

  /** Map a linear character offset back to a caret in `root` (clamped to end). */
  private caretAtLinear(root: HTMLElement, target: number): CaretPos | null {
    let acc = 0;
    let last: Text | null = null;
    for (const n of allTextNodes(root)) {
      if (target <= acc + n.data.length) return { node: n, offset: target - acc };
      acc += n.data.length;
      last = n;
    }
    return last ? { node: last, offset: last.data.length } : null;
  }

  /** Snapshot the live selection as linear character offsets over the current
   *  root's text stream, so it can be re-applied to another root that renders
   *  the same text (side-panel preview ⇄ quicklook). Read this BEFORE the target
   *  root exists - the source root's text may unmount as the target mounts. */
  captureLinear(): { anchor: number | null; focus: number } | null {
    if (!this.root || !this.focus) return null;
    const focus = this.linearOffset(this.focus);
    if (focus === null) return null;
    const anchor = this.anchor ? this.linearOffset(this.anchor) : null;
    return { anchor, focus };
  }

  /** Re-apply a captured selection onto `root`, remapping offsets to its text.
   *  Returns false (leaving the selection untouched) if the target has no text
   *  yet - async previews populate a frame or two after mount, so the caller
   *  retries. Keyboard mode is preserved. */
  applyLinear(root: HTMLElement, cap: { anchor: number | null; focus: number }): boolean {
    if (allTextNodes(root).length === 0) return false;
    const newFocus = this.caretAtLinear(root, cap.focus);
    if (!newFocus) return false;
    const newAnchor = cap.anchor !== null ? this.caretAtLinear(root, cap.anchor) : null;
    this.cancelDrag();
    this.disconnectObserver();
    this.root = root;
    this.focus = newFocus;
    this.anchor = newAnchor;
    this.observeRoot(root);
    this.emit();
    return true;
  }

  private exitKeyboard(): void {
    if (!this.keyboard) return;
    this.keyboard = false;
    this.goalX = null;
    window.removeEventListener("keydown", this.onKeyboardKey, true);
  }

  /** Sticky column for Up/Down runs (reset by horizontal movement). */
  private goalX: number | null = null;

  private onKeyboardKey = (e: KeyboardEvent) => {
    if (!this.keyboard || !this.root?.isConnected || !this.focus?.node.isConnected) {
      this.clear();
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      this.clear();
      return;
    }
    const move = MOVE_KEYS[e.key];
    // Everything else (Ctrl+C, typing, Enter, …) passes through untouched.
    if (!move || e.altKey || e.metaKey) return;
    e.preventDefault();
    e.stopPropagation();

    // Shift extends from a fixed anchor; unshifted movement collapses.
    if (e.shiftKey) {
      if (!this.anchor) this.anchor = this.focus;
    } else {
      this.anchor = null;
    }

    let next: CaretPos | null = null;
    switch (move) {
      case "left":
        next = e.ctrlKey ? this.wordStep(-1) : this.charStep(-1);
        this.goalX = null;
        break;
      case "right":
        next = e.ctrlKey ? this.wordStep(1) : this.charStep(1);
        this.goalX = null;
        break;
      case "up":
        next = this.lineStep(-1);
        break;
      case "down":
        next = this.lineStep(1);
        break;
      case "home":
        next = this.lineEdge(-1);
        this.goalX = null;
        break;
      case "end":
        next = this.lineEdge(1);
        this.goalX = null;
        break;
    }
    if (next) {
      this.focus = next;
      this.scrollCaretIntoView();
      this.emit();
    }
  };

  private charStep(dir: -1 | 1): CaretPos | null {
    const { node, offset } = this.focus!;
    if (dir > 0 && offset < node.data.length) return { node, offset: offset + 1 };
    if (dir < 0 && offset > 0) return { node, offset: offset - 1 };
    // Hop to the neighboring text node.
    const nodes = allTextNodes(this.root!);
    let i = nodes.indexOf(node) + dir;
    while (i >= 0 && i < nodes.length) {
      const n = nodes[i];
      if (n.data.length > 0) {
        return dir > 0 ? { node: n, offset: 1 } : { node: n, offset: n.data.length - 1 };
      }
      i += dir;
    }
    return null;
  }

  /** The character the caret would cross moving one step in `dir`. */
  private charAt(pos: CaretPos, dir: -1 | 1): string | null {
    const { node, offset } = pos;
    if (dir > 0) {
      if (offset < node.data.length) return node.data[offset];
    } else if (offset > 0) {
      return node.data[offset - 1];
    }
    const nodes = allTextNodes(this.root!);
    let i = nodes.indexOf(node) + dir;
    while (i >= 0 && i < nodes.length) {
      const n = nodes[i];
      if (n.data.length > 0) return dir > 0 ? n.data[0] : n.data[n.data.length - 1];
      i += dir;
    }
    return null;
  }

  private wordStep(dir: -1 | 1): CaretPos | null {
    let pos = this.focus!;
    // Skip separators, then run to the end of the word.
    for (;;) {
      const c = this.charAt(pos, dir);
      if (c === null) return pos === this.focus ? null : pos;
      if (WORD_CHAR.test(c)) break;
      const step = this.stepFrom(pos, dir);
      if (!step) return pos === this.focus ? null : pos;
      pos = step;
    }
    for (;;) {
      const c = this.charAt(pos, dir);
      if (c === null || !WORD_CHAR.test(c)) return pos;
      const step = this.stepFrom(pos, dir);
      if (!step) return pos;
      pos = step;
    }
  }

  private stepFrom(pos: CaretPos, dir: -1 | 1): CaretPos | null {
    const saved = this.focus;
    this.focus = pos;
    const out = this.charStep(dir);
    this.focus = saved;
    return out;
  }

  /** Vertical move by one visual line. Deterministic line-band model: collect
   *  every text node's line-fragment rects, find the band directly above/below
   *  the caret's own line, then land at the sticky goal column within it. No
   *  pixel-probing (fragile on variable line heights - headings, mixed fonts). */
  private lineStep(dir: -1 | 1): CaretPos | null {
    const caret = caretClientRect(this.focus!);
    if (!caret) return null;
    if (this.goalX === null) this.goalX = caret.left;
    const goalX = this.goalX;
    const caretMidY = caret.top + caret.height / 2;

    // One rect per wrapped line-fragment of each non-empty text node.
    const rows: { node: Text; rect: DOMRect }[] = [];
    const r = document.createRange();
    for (const node of allTextNodes(this.root!)) {
      if (node.data.length === 0) continue;
      r.selectNodeContents(node);
      for (const rect of r.getClientRects()) {
        if (rect.width <= 0 || rect.height <= 0) continue;
        rows.push({ node, rect });
      }
    }

    // Closest row strictly on the far side of the caret's line (its center past
    // the caret's line in `dir`). Ties broken by horizontal nearness to goalX.
    let seed: { node: Text; rect: DOMRect } | null = null;
    let seedGap = Infinity;
    for (const row of rows) {
      const midY = row.rect.top + row.rect.height / 2;
      const delta = (midY - caretMidY) * dir;
      // Require the row's center to clear half the caret's height, so fragments
      // sharing the caret's own line are excluded even when heights differ.
      if (delta <= caret.height * 0.5) continue;
      const hDist = goalX < row.rect.left ? row.rect.left - goalX
        : goalX > row.rect.right ? goalX - row.rect.right : 0;
      if (delta < seedGap - 0.5 || (Math.abs(delta - seedGap) <= 0.5 && seed
        && hDist < this.rowHDist(seed, goalX))) {
        seed = row;
        seedGap = delta;
      }
    }
    if (!seed) return null;

    // The target line = all fragments overlapping the seed row vertically.
    let best = seed;
    let bestDist = this.rowHDist(seed, goalX);
    for (const row of rows) {
      const overlap = Math.min(row.rect.bottom, seed.rect.bottom)
        - Math.max(row.rect.top, seed.rect.top);
      if (overlap <= 0.5 * Math.min(row.rect.height, seed.rect.height)) continue;
      const d = this.rowHDist(row, goalX);
      if (d < bestDist) { bestDist = d; best = row; }
    }

    const y = best.rect.top + best.rect.height / 2;
    return { node: best.node, offset: offsetAtX(best.node, goalX, y) };
  }

  /** Horizontal distance from goalX to a row's rect (0 when inside it). */
  private rowHDist(row: { rect: DOMRect }, goalX: number): number {
    if (goalX < row.rect.left) return row.rect.left - goalX;
    if (goalX > row.rect.right) return goalX - row.rect.right;
    return 0;
  }

  private lineEdge(dir: -1 | 1): CaretPos | null {
    const rect = caretClientRect(this.focus!);
    if (!rect) return null;
    const rootRect = this.root!.getBoundingClientRect();
    const x = dir < 0 ? rootRect.left + 2 : rootRect.right - 2;
    return caretFromPoint(x, rect.top + rect.height / 2, this.root!);
  }

  private scrollCaretIntoView(): void {
    const rect = this.focus ? caretClientRect(this.focus) : null;
    const scroller = this.root ? scrollableAncestor(this.root) : null;
    if (!rect || !scroller) return;
    const view = scroller.getBoundingClientRect();
    const pad = 12;
    if (rect.top < view.top + pad) scroller.scrollTop -= view.top + pad - rect.top;
    else if (rect.bottom > view.bottom - pad) scroller.scrollTop += rect.bottom - (view.bottom - pad);
    if (rect.left < view.left + pad) scroller.scrollLeft -= view.left + pad - rect.left;
    else if (rect.right > view.right - pad) scroller.scrollLeft += rect.right - (view.right - pad);
  }

  /** The current selection as a forward DOM Range, or null. */
  range(): Range | null {
    const { anchor, focus } = this;
    if (!anchor || !focus || !anchor.node.isConnected || !focus.node.isConnected) return null;
    const r = document.createRange();
    try {
      r.setStart(anchor.node, anchor.offset);
      r.setEnd(focus.node, focus.offset);
    } catch {
      return null;
    }
    if (!r.collapsed) return r;
    if (anchor.node === focus.node && anchor.offset === focus.offset) return null;
    // Backwards drag: the browser collapsed the range; flip it.
    const rev = document.createRange();
    try {
      rev.setStart(focus.node, focus.offset);
      rev.setEnd(anchor.node, anchor.offset);
    } catch {
      return null;
    }
    return rev.collapsed ? null : rev;
  }

  // ── mouse tracking ──────────────────────────────────────────────────────────

  private onMouseDown = (e: MouseEvent) => {
    this.suppressClick = false;
    if (e.button !== 0) return;
    const target = e.target as HTMLElement;
    // The popover acts on the selection - clicking it must not clear it.
    if (target.closest(".sel-popover")) return;
    const root = target.closest<HTMLElement>("[data-selectable]");
    if (!root) {
      this.clear();
      return;
    }
    if (e.detail >= 2) {
      // Double-click selects the word, triple-click the visual/logical line.
      // A drag afterwards extends the selection at that same granularity,
      // anchored on the initially hit word/line.
      const caret = caretFromPoint(e.clientX, e.clientY, root);
      const gran = e.detail === 2 ? "word" : "line";
      const bounds = caret ? this.granularBounds(caret, root, gran) : null;
      if (bounds) {
        this.setSelection(root, bounds[0], bounds[1]);
        this.pending = { root, x: e.clientX, y: e.clientY };
        this.dragGranularity = gran;
        this.dragAnchorBounds = bounds;
        window.addEventListener("mousemove", this.onMouseMove);
        window.addEventListener("mouseup", this.onMouseUp);
      }
      return;
    }
    this.pending = { root, x: e.clientX, y: e.clientY };
    this.dragGranularity = "char";
    this.dragAnchorBounds = null;
    window.addEventListener("mousemove", this.onMouseMove);
    window.addEventListener("mouseup", this.onMouseUp);
  };

  /** [start, end) bounds for a click at `caret` under the given granularity. */
  private granularBounds(
    caret: CaretPos,
    root: HTMLElement,
    gran: "char" | "word" | "line",
  ): [CaretPos, CaretPos] | null {
    if (gran === "word") return this.wordCaretBounds(caret);
    if (gran === "line") return this.lineCaretBounds(caret, root);
    return [caret, caret];
  }

  private wordCaretBounds(caret: CaretPos): [CaretPos, CaretPos] | null {
    const bounds = wordBoundsAt(caret.node.data, caret.offset);
    if (!bounds) return null;
    return [{ node: caret.node, offset: bounds[0] }, { node: caret.node, offset: bounds[1] }];
  }

  /** Line select: expand to the nearest newline / block boundary on each side,
   *  crossing text nodes (syntax-highlight spans split lines into many nodes). */
  private lineCaretBounds(caret: CaretPos, root: HTMLElement): [CaretPos, CaretPos] | null {
    const nodes = allTextNodes(root);
    const idx = nodes.indexOf(caret.node);
    if (idx < 0) return null;

    let start: CaretPos = { node: nodes[0], offset: 0 };
    const nlBefore = caret.node.data.lastIndexOf("\n", Math.max(0, caret.offset - 1));
    if (nlBefore >= 0 && nlBefore < caret.offset) {
      start = { node: caret.node, offset: nlBefore + 1 };
    } else {
      for (let i = idx - 1; i >= 0; i--) {
        if (separatesLine(nodes[i], nodes[i + 1], root)) {
          start = { node: nodes[i + 1], offset: 0 };
          break;
        }
        const p = nodes[i].data.lastIndexOf("\n");
        if (p >= 0) {
          start = { node: nodes[i], offset: p + 1 };
          break;
        }
      }
    }

    let end: CaretPos = { node: nodes[nodes.length - 1], offset: nodes[nodes.length - 1].data.length };
    const nlAfter = caret.node.data.indexOf("\n", caret.offset);
    if (nlAfter >= 0) {
      end = { node: caret.node, offset: nlAfter };
    } else {
      for (let i = idx + 1; i < nodes.length; i++) {
        if (separatesLine(nodes[i - 1], nodes[i], root)) {
          end = { node: nodes[i - 1], offset: nodes[i - 1].data.length };
          break;
        }
        const p = nodes[i].data.indexOf("\n");
        if (p >= 0) {
          end = { node: nodes[i], offset: p };
          break;
        }
      }
    }

    return [start, end];
  }

  /** Install a programmatic (non-drag) selection and publish it. */
  private setSelection(root: HTMLElement, anchor: CaretPos, focus: CaretPos): void {
    this.cancelDrag();
    this.keyboard = false;
    this.root = root;
    this.anchor = anchor;
    this.focus = focus;
    this.observeRoot(root);
    this.emit();
  }

  private onMouseMove = (e: MouseEvent) => {
    const pending = this.pending;
    if (!pending) return;
    if (!this.dragging) {
      if (Math.hypot(e.clientX - pending.x, e.clientY - pending.y) < DRAG_THRESHOLD) return;
      if (this.dragGranularity === "char") {
        const anchor = caretFromPoint(pending.x, pending.y, pending.root);
        if (!anchor) return;
        // A new drag replaces any previous selection wholesale.
        this.root = pending.root;
        this.anchor = anchor;
        this.focus = anchor;
        this.observeRoot(pending.root);
      } else {
        // Granular drag: the word/line selection is already installed by the
        // mousedown; keep it and extend from here.
        this.root = pending.root;
      }
      this.dragging = true;
      this.keyboard = false;
    }
    if (!this.root?.isConnected) {
      this.clear();
      return;
    }
    const focus = caretFromPoint(e.clientX, e.clientY, this.root);
    if (focus) {
      if (this.dragGranularity === "char" || !this.dragAnchorBounds) {
        this.focus = focus;
      } else {
        // Extend by whole words/lines: union the anchor bounds with the bounds
        // of the word/line under the pointer, keeping the far edge fixed.
        const fb = this.granularBounds(focus, this.root, this.dragGranularity) ?? [focus, focus];
        const [aLo, aHi] = this.dragAnchorBounds;
        if (beforeOrEqual(fb[1], aLo)) {
          this.anchor = aHi;
          this.focus = fb[0];
        } else {
          this.anchor = aLo;
          this.focus = fb[1];
        }
      }
    }
    this.emit();
  };

  private onMouseUp = () => {
    const wasDragging = this.dragging;
    const wasGranular = this.dragGranularity !== "char";
    this.pending = null;
    this.dragging = false;
    this.dragGranularity = "char";
    this.dragAnchorBounds = null;
    window.removeEventListener("mousemove", this.onMouseMove);
    window.removeEventListener("mouseup", this.onMouseUp);
    if (!wasDragging) {
      // A double/triple-click that never dragged already installed a word/line
      // selection - keep it. A plain single click clears, like a native click.
      if (!wasGranular) this.clear();
      return;
    }
    if (!this.range()) {
      this.clear();
      return;
    }
    // A drag that selected text must not also activate what's under the
    // pointer (links in markdown previews). Cleared by the click that follows,
    // or by the next mousedown if none does (released off-window).
    this.suppressClick = true;
    this.emit();
  };

  private onClickCapture = (e: MouseEvent) => {
    if (!this.suppressClick) return;
    this.suppressClick = false;
    e.preventDefault();
    e.stopPropagation();
  };

  private cancelDrag(): void {
    this.dragGranularity = "char";
    this.dragAnchorBounds = null;
    if (!this.pending && !this.dragging) return;
    this.pending = null;
    this.dragging = false;
    window.removeEventListener("mousemove", this.onMouseMove);
    window.removeEventListener("mouseup", this.onMouseUp);
  }

  // ── invalidation ────────────────────────────────────────────────────────────

  private observeRoot(root: HTMLElement): void {
    this.disconnectObserver();
    this.observer = new MutationObserver(() => {
      if (!this.root?.isConnected || (this.anchor && !this.anchor.node.isConnected)
        || (this.focus && !this.focus.node.isConnected)) {
        this.clear();
      }
    });
    this.observer.observe(root, { childList: true, subtree: true, characterData: true });
    // A layout resize (panel/window) invalidates content-space rects; scroll
    // does NOT need a recompute - the overlay scrolls with the content. Escape
    // or a new selection is the recovery for a post-resize stale selection.
    window.addEventListener("resize", this.onReflow);
  }

  private onReflow = () => {
    if (this.root) this.emit();
  };

  private disconnectObserver(): void {
    this.observer?.disconnect();
    this.observer = null;
    window.removeEventListener("resize", this.onReflow);
  }

  // ── snapshot ────────────────────────────────────────────────────────────────

  private emit(): void {
    const root = this.root;
    const range = root ? this.range() : null;
    if (!root || (!range && !this.keyboard)) {
      this.snapshot = EMPTY;
    } else {
      // Publish the live range + focus; SelectionLayer measures rects from them
      // relative to the overlay's own box (correct whether the overlay scrolls
      // or pins) and recomputes on scroll.
      this.snapshot = {
        root,
        range,
        focus: this.focus,
        dragging: this.dragging,
        keyboard: this.keyboard,
      };
    }
    this.listeners.forEach(l => l());
  }
}

export const selection = new SelectionController();
