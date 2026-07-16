import { ReactNode, useRef, useState } from "react";

/** Per-row context handed to `renderRow`. Spread `handleProps` onto the grip. */
export interface SortableRowCtx {
  /** This row is being dragged. */
  dragging: boolean;
  expanded: boolean;
  toggleExpand: () => void;
  handleProps: {
    onPointerDown: (e: React.PointerEvent) => void;
    onKeyDown: (e: React.KeyboardEvent) => void;
    tabIndex: 0;
    role: "button";
    "aria-label": string;
  };
}

interface Props<T> {
  items: T[];
  getKey: (item: T) => string;
  /** Fired on drop and on every keyboard move with the full new key order. */
  onReorder: (keys: string[]) => void;
  renderRow: (item: T, ctx: SortableRowCtx) => ReactNode;
  /** Presence enables the expandable detail slot under a row. */
  renderExpanded?: (item: T) => ReactNode;
  ariaLabel: string;
  /** Extra class on the list root, for scoping consumer-specific styling. */
  className?: string;
}

interface DragState {
  key: string;
  index: number;
  startY: number;
  /** Pointer offset in px from the drag start. */
  dy: number;
  rowH: number;
}

/**
 * Shared drag-to-reorder list: pointer-capture drag on a grip, uniform-height
 * slot math, keyboard Arrow/Home/End moves, `aria-live` announcements. DOM
 * order never changes mid-drag - the lifted row follows the pointer via a
 * transform and displaced rows glide by one slot; on drop the transforms are
 * cleared in the same commit that reorders, so nothing snaps.
 */
export default function SortableList<T>({ items, getKey, onReorder, renderRow, renderExpanded, ariaLabel, className }: Props<T>) {
  const [drag, setDrag] = useState<DragState | null>(null);
  const [expandedKey, setExpandedKey] = useState<string | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const dragRef = useRef<DragState | null>(null);
  const keys = items.map(getKey);

  const announce = (key: string, to: number) => {
    setAnnouncement(`${key} moved to position ${to + 1} of ${keys.length}`);
  };

  const moveTo = (from: number, to: number) => {
    if (to < 0 || to >= keys.length || to === from) return;
    const next = [...keys];
    const [k] = next.splice(from, 1);
    next.splice(to, 0, k);
    onReorder(next);
    announce(k, to);
  };

  /** Slot the lifted row currently hovers, clamped to the list. */
  const dropIndex = (d: DragState) => {
    const raw = d.index + Math.round(d.dy / d.rowH);
    return Math.max(0, Math.min(keys.length - 1, raw));
  };

  const startDrag = (key: string, index: number) => (e: React.PointerEvent) => {
    // Uniform slot math needs uniform rows: collapse any open detail first.
    setExpandedKey(null);
    const row = (e.currentTarget as HTMLElement).closest<HTMLElement>(".settings-sortable-row");
    if (!row) return;
    const rowH = row.offsetHeight;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    const d: DragState = { key, index, startY: e.clientY, dy: 0, rowH };
    dragRef.current = d;
    setDrag(d);

    const grip = e.currentTarget as HTMLElement;
    const onMove = (ev: PointerEvent) => {
      const cur = dragRef.current;
      if (!cur) return;
      const next = { ...cur, dy: ev.clientY - cur.startY };
      dragRef.current = next;
      setDrag(next);
    };
    const finish = (commit: boolean) => {
      grip.removeEventListener("pointermove", onMove);
      grip.removeEventListener("pointerup", onUp);
      grip.removeEventListener("pointercancel", onCancel);
      const cur = dragRef.current;
      dragRef.current = null;
      setDrag(null);
      if (commit && cur) moveTo(cur.index, dropIndex(cur));
    };
    const onUp = () => finish(true);
    const onCancel = () => finish(false);
    grip.addEventListener("pointermove", onMove);
    grip.addEventListener("pointerup", onUp);
    grip.addEventListener("pointercancel", onCancel);
  };

  const keyMove = (index: number) => (e: React.KeyboardEvent) => {
    const to =
      e.key === "ArrowUp" ? index - 1 :
      e.key === "ArrowDown" ? index + 1 :
      e.key === "Home" ? 0 :
      e.key === "End" ? keys.length - 1 :
      null;
    if (to === null) return;
    e.preventDefault();
    moveTo(index, to);
  };

  return (
    <div className={`settings-sortable${className ? ` ${className}` : ""}`} role="list" aria-label={ariaLabel}>
      {items.map((item, i) => {
        const key = getKey(item);
        const lifted = drag?.key === key;
        // Rows the lifted one has passed shift one slot toward its origin.
        let shift = 0;
        if (drag && !lifted) {
          const target = dropIndex(drag);
          if (drag.index < i && target >= i) shift = -drag.rowH;
          else if (drag.index > i && target <= i) shift = drag.rowH;
        }
        const expanded = expandedKey === key && !drag;
        const ctx: SortableRowCtx = {
          dragging: !!lifted,
          expanded,
          toggleExpand: () => setExpandedKey(k => (k === key ? null : key)),
          handleProps: {
            onPointerDown: startDrag(key, i),
            onKeyDown: keyMove(i),
            tabIndex: 0,
            role: "button",
            "aria-label": `Reorder ${key}`,
          },
        };
        return (
          <div key={key} role="listitem">
            <div
              className={`settings-sortable-row${lifted ? " lifted" : ""}`}
              style={
                lifted
                  ? { transform: `translateY(${drag!.dy}px)` }
                  : shift !== 0
                    ? { transform: `translateY(${shift}px)` }
                    : undefined
              }
            >
              {renderRow(item, ctx)}
            </div>
            {expanded && renderExpanded && (
              <div className="settings-sortable-detail">{renderExpanded(item)}</div>
            )}
          </div>
        );
      })}
      <div className="settings-sortable-live" aria-live="polite">{announcement}</div>
    </div>
  );
}
