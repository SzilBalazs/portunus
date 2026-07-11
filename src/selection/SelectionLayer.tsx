// Renders the virtual selection: highlight rects, keyboard caret, and popover,
// portaled INTO the active selectable root.
//
// Rects are measured here (not in the controller) because they must be relative
// to the OVERLAY's own live bounding box: WebKitGTK inconsistently scrolls vs
// pins absolutely-positioned children of a scroll container, so anchoring to the
// overlay's actual painted position is correct either way. A scroll/resize
// listener recomputes while a selection is live.

import { Fragment, useEffect, useLayoutEffect, useReducer, useRef, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { selection } from "./controller";
import { caretRect, rectsForRange, rootScale, type SelRect } from "./geometry";
import SelectionPopover, { type PopoverActions } from "./SelectionPopover";

export default function SelectionLayer({ actions }: { actions: PopoverActions }) {
  const snap = useSyncExternalStore(selection.subscribe, selection.getSnapshot);
  const overlayRef = useRef<HTMLDivElement>(null);
  // Force a recompute after mount (overlay ref lands) and on scroll/resize.
  const [, bump] = useReducer((n: number) => n + 1, 0);

  const active = !!snap.root && (!!snap.range || snap.keyboard);

  // Hold the caret solid while it's actively moving (key-repeat) - the blink
  // makes a moving cursor hard to track. Blink resumes ~500ms after the last move.
  const [moving, setMoving] = useState(false);
  const moveTimer = useRef<number>(0);
  useEffect(() => {
    if (!snap.keyboard || !snap.focus) return;
    setMoving(true);
    clearTimeout(moveTimer.current);
    moveTimer.current = window.setTimeout(() => setMoving(false), 500);
    return () => clearTimeout(moveTimer.current);
  }, [snap.keyboard, snap.focus]);

  useLayoutEffect(() => {
    if (!active) return;
    bump(); // measure once the overlay is in the DOM
    const on = () => bump();
    // Capture-phase scroll catches inner scroll containers (events don't bubble).
    window.addEventListener("scroll", on, true);
    window.addEventListener("resize", on);
    return () => {
      window.removeEventListener("scroll", on, true);
      window.removeEventListener("resize", on);
    };
  }, [active, snap.range, snap.keyboard]);

  if (!snap.root || !active) return null;

  // Measure relative to the overlay's own box (see file header).
  let rects: SelRect[] = [];
  let caret: SelRect | null = null;
  let focusRect: SelRect | null = null;
  // Visible box (the scroll viewport) expressed in the same overlay-local space,
  // so the popover can flip/clamp regardless of scroll/pin behavior.
  let viewport = { top: 0, bottom: 0, left: 0, right: 0 };
  const overlay = overlayRef.current;
  if (overlay) {
    const originRect = overlay.getBoundingClientRect();
    const scale = rootScale(snap.root);
    if (snap.range) rects = rectsForRange(snap.range, originRect, scale);
    if (snap.focus) {
      const c = caretRect(snap.focus, originRect, scale);
      // Anchor the popover at the focus end (drag-release / caret), not the
      // bottom-most rect (wrong end for an upward selection).
      focusRect = c ?? (rects.length > 0 ? rects[rects.length - 1] : null);
      if (snap.keyboard) caret = c;
    }
    const vb = snap.root.getBoundingClientRect();
    viewport = {
      top: (vb.top - originRect.top) / scale.y,
      bottom: (vb.bottom - originRect.top) / scale.y,
      left: (vb.left - originRect.left) / scale.x,
      right: (vb.right - originRect.left) / scale.x,
    };
  }

  const showPopover = !snap.dragging && !snap.keyboard && rects.length > 0 && focusRect;

  return createPortal(
    <Fragment>
      <div ref={overlayRef} className="sel-overlay" aria-hidden="true">
        {rects.map((r, i) => (
          <div key={i} className="sel-rect" style={{ left: r.x, top: r.y, width: r.w, height: r.h }} />
        ))}
        {caret && (
          <div className="sel-caret" data-moving={moving || undefined} style={{ left: caret.x, top: caret.y, height: caret.h }} />
        )}
      </div>
      {showPopover && (
        <SelectionPopover
          key={`${focusRect!.x},${focusRect!.y}`}
          anchor={focusRect!}
          viewport={viewport}
          actions={actions}
        />
      )}
    </Fragment>,
    snap.root,
  );
}
