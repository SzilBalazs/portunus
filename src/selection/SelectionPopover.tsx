// Floating action pill shown near the end of a completed selection:
// Copy · Search · one smart entity chip (calc result / define / open).
// Lives inside the portaled overlay, so it scrolls with the selected text.
// Never steals focus: divs only, mousedown prevented at the container.

import { useLayoutEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SelRect } from "./geometry";
import { selection } from "./controller";
import { detectEntity, type Entity } from "./entity";

export interface PopoverActions {
  onSearch: (text: string) => void;
  onDefine: (word: string) => void;
}

const EDGE_PAD = 6;
const GAP = 8;

function openExternal(target: string) {
  // launch_app tokenizes the exec string (argv, not a shell), so a double-quote
  // in the target would mis-tokenize the argument. URLs/emails with a quote are
  // degenerate; drop them rather than open a garbled target.
  if (target.includes('"')) return;
  // Same backend route markdown links use (xdg-open + hide).
  invoke("launch_app", { exec: `xdg-open "${target}"` })
    .catch(err => console.error("[selection] open failed:", err));
}

export interface Viewport {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

export default function SelectionPopover({
  anchor,
  viewport,
  actions,
}: {
  anchor: SelRect;
  /** Visible scroll-viewport in the same (overlay-local) space as `anchor`. */
  viewport: Viewport;
  actions: PopoverActions;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const [copied, setCopied] = useState(false);
  const [entity, setEntity] = useState<Entity | null>(null);
  const [mathResult, setMathResult] = useState<string | null>(null);

  // Classify once per selection (the popover remounts via key= on the anchor).
  useLayoutEffect(() => {
    const text = selection.getText();
    const ent = detectEntity(text);
    setEntity(ent);
    if (ent?.type === "math") {
      let stale = false;
      invoke<string | null>("calc_eval", { expr: ent.value })
        .then(res => { if (!stale) setMathResult(res); })
        .catch(() => {});
      return () => { stale = true; };
    }
  }, []);

  // Position after first (hidden) paint. anchor and viewport are in the same
  // overlay-local space: below the anchor, flipped above near the visible
  // bottom, clamped to the visible box on both axes.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    let top = anchor.y + anchor.h + GAP;
    if (top + h > viewport.bottom - EDGE_PAD) top = anchor.y - h - GAP;
    top = Math.min(Math.max(top, viewport.top + EDGE_PAD), Math.max(viewport.top + EDGE_PAD, viewport.bottom - h - EDGE_PAD));
    const maxLeft = Math.max(viewport.left + EDGE_PAD, viewport.right - w - EDGE_PAD);
    const left = Math.min(Math.max(anchor.x, viewport.left + EDGE_PAD), maxLeft);
    setPos({ left, top });
  }, [anchor.x, anchor.y, anchor.h, viewport.top, viewport.bottom, viewport.left, viewport.right, mathResult, entity]);

  const copySelection = () => {
    selection.copy();
    setCopied(true);
    setTimeout(() => setCopied(false), 900);
  };

  const copyMathResult = () => {
    if (mathResult == null) return;
    invoke("copy_text", { text: mathResult }).catch(() => {
      navigator.clipboard.writeText(mathResult).catch(() => {});
    });
    selection.clear();
  };

  // Act on mousedown, not click: a drag that crosses elements fires no trailing
  // `click` (so the controller's suppress-click guard can eat the next one), and
  // running the action here also dodges any unmount race between down and click.
  // preventDefault keeps focus on the search input; stopPropagation keeps the
  // controller's document-level mousedown from treating it as an outside click.
  const act = (fn: () => void) => (e: ReactMouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    fn();
  };

  const chip = (() => {
    if (!entity) return null;
    switch (entity.type) {
      case "math":
        return mathResult != null ? (
          <div className="sel-popover-btn sel-popover-entity" onMouseDown={act(copyMathResult)} title="Copy result">
            = {mathResult}
          </div>
        ) : null;
      case "word":
        return (
          <div className="sel-popover-btn sel-popover-entity" onMouseDown={act(() => actions.onDefine(entity.value))}>
            Define
          </div>
        );
      case "url":
      case "email":
        return (
          <div
            className="sel-popover-btn sel-popover-entity"
            onMouseDown={act(() => openExternal(entity.type === "email" ? `mailto:${entity.value}` : entity.value))}
          >
            Open
          </div>
        );
    }
  })();

  return (
    <div
      ref={ref}
      className="sel-popover"
      style={pos ? { left: pos.left, top: pos.top } : { left: anchor.x, top: anchor.y, visibility: "hidden" }}
      onMouseDown={e => e.preventDefault()}
    >
      <div className="sel-popover-btn" onMouseDown={act(copySelection)}>{copied ? "Copied" : "Copy"}</div>
      <div className="sel-popover-btn" onMouseDown={act(() => actions.onSearch(selection.getText()))}>Search</div>
      {chip}
    </div>
  );
}
