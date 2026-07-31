import { Fragment, memo, useCallback, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { SearchResult } from "../types";
import { groupLabel, shortenPath } from "../utils";
import { PinIcon } from "../icons";
import ResultIcon from "./ResultIcon";

interface RowProps {
  result: SearchResult;
  index: number;
  isSelected: boolean;
  /** Group header text, or null when this row continues the previous group. */
  label: string | null;
  /** Alt+N shortcut digit, or 0 for rows past the ninth launchable one. */
  shortcut: number;
  rowRef: (el: HTMLElement | null) => void;
  /** Only supplied when this row renders a header. */
  labelRef?: (el: HTMLElement | null) => void;
  onSelect: (index: number) => void;
  onLaunch: (result: SearchResult) => void;
}

/**
 * One result row plus (optionally) the group header above it.
 *
 * Memoized, and the reason is load-bearing: a scope list is not truncated to
 * `max_results` (browse mode shows everything), so an unmemoized list re-rendered
 * every whole subtree on every arrow keypress — tens of ms of commit for a move
 * that only ever changes two rows. Every prop here is therefore either primitive
 * or referentially stable across renders; adding an inline callback or a freshly
 * built object to this signature silently undoes that.
 */
const ResultRow = memo(function ResultRow({
  result, index, isSelected, label, shortcut, rowRef, labelRef, onSelect, onLaunch,
}: RowProps) {
  return (
    <Fragment>
      {label !== null && (
        <div ref={labelRef} className={`result-group-label${index === 0 ? " first" : ""}`}>
          <span>{label}</span>
        </div>
      )}
      <div
        ref={rowRef}
        className={`result-row${isSelected ? " selected" : ""}`}
        data-kind={result.kind}
        style={{ '--row-i': index } as CSSProperties}
        role="option"
        aria-selected={isSelected}
        onClick={() => { onSelect(index); onLaunch(result); }}
      >
        <ResultIcon icon_path={result.icon_path} iconDataUri={result.icon_data_uri} glyph={result.command?.glyph} title={result.title} kind={result.kind} />
        <div className="result-text">
          <div className="result-title">{result.title}</div>
          {result.subtitle && <div className="result-subtitle">{shortenPath(result.subtitle)}</div>}
        </div>
        <div className={`result-meta${result.ext?.badge ? " has-badge" : ""}${result.pinned ? " has-pin" : ""}`}>
          {result.pinned && (
            <span className="result-pin" title="Pinned for this search" aria-label="Pinned">
              <PinIcon />
            </span>
          )}
          {result.ext?.badge
            ? <span className="result-badge">{result.ext.badge}</span>
            : ""}
        </div>
        <div className="result-shortcut" style={shortcut === 0 ? { visibility: 'hidden' } : undefined}>
          {shortcut === 0 ? "" : shortcut}
        </div>
      </div>
    </Fragment>
  );
});

interface Props {
  results: SearchResult[];
  selectedIndex: number;
  /** Whether the user has entered a meaningful search term (drives the empty state). */
  active: boolean;
  searching?: boolean;
  onSelect: (index: number) => void;
  onLaunch: (result?: SearchResult) => void;
  launchableResults: SearchResult[];
  /** Empty-state text when a search resolves with no results. */
  emptyLabel?: string;
  /** Gate for showing the empty state — held false until the empty verdict has
   *  settled, so a dead prefix mid-typing stays blank instead of flashing. */
  emptyReady?: boolean;
  /** Names of extensions whose async query is still running - rendered as
   *  slim loading rows below the results. */
  pending?: string[];
}

export default function ResultsList({ results, selectedIndex, active, searching, onSelect, onLaunch, launchableResults, emptyLabel = "No results", emptyReady = true, pending = [] }: Props) {
  const colRef = useRef<HTMLDivElement>(null);
  // FLIP: live element nodes keyed by a stable flip-key, plus their last-known
  // offsetTop and any in-flight glide. When the result set re-ranks, retained
  // elements glide from old to new position instead of snapping. Rows are keyed
  // by result id ("r:<id>"); group labels by their anchoring result id
  // ("g:<id>") — keying by label text would collide when the same kind appears
  // in two separate groups (e.g. two "APPS" headers). Only active when
  // appearance.animate_results === "flip".
  const flipEls = useRef<Map<string, HTMLElement>>(new Map());
  const flipTops = useRef<Map<string, number>>(new Map());
  const flipAnims = useRef<Map<string, Animation>>(new Map());

  // One ref callback per flip-key, cached and reused. An inline `el => …` would
  // be a fresh function identity every render, so React would detach and reattach
  // *every* row's ref on every keypress (and churn the flip maps doing it) —
  // and no memoized row could ever bail out, since its ref prop always differs.
  const flipRefFns = useRef<Map<string, (el: HTMLElement | null) => void>>(new Map());
  const flipRef = useCallback((key: string) => {
    let fn = flipRefFns.current.get(key);
    if (!fn) {
      fn = (el: HTMLElement | null) => {
        if (el) {
          flipEls.current.set(key, el);
        } else {
          flipEls.current.delete(key);
          flipTops.current.delete(key);
        }
      };
      flipRefFns.current.set(key, fn);
    }
    return fn;
  }, []);

  // Group header per row (null = continues the previous group), resolved once per
  // result set. Both the rows and the scroll-anchor logic below read it.
  const labels = useMemo(
    () => results.map((r, i) => {
      const label = groupLabel(r.kind);
      if (label === null) return null;
      return i > 0 && label === groupLabel(results[i - 1].kind) ? null : label;
    }),
    [results],
  );

  // Alt+N digit per result id. Built once per result set instead of an
  // `indexOf` per row (which made rendering the list quadratic).
  const shortcuts = useMemo(() => {
    const m = new Map<string, number>();
    launchableResults.forEach((r, i) => { if (i < 9) m.set(r.id, i + 1); });
    return m;
  }, [launchableResults]);

  // Row callbacks reach the current props through refs, so the identities handed
  // to the memoized rows never change even when the parent passes fresh closures.
  const onSelectRef = useRef(onSelect);
  const onLaunchRef = useRef(onLaunch);
  onSelectRef.current = onSelect;
  onLaunchRef.current = onLaunch;
  const handleSelect = useCallback((i: number) => onSelectRef.current(i), []);
  const handleLaunch = useCallback((r: SearchResult) => onLaunchRef.current(r), []);

  useLayoutEffect(() => {
    const flip =
      document.documentElement.dataset.animateResults === "flip" &&
      !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const next = new Map<string, number>();
    for (const [key, el] of flipEls.current) {
      const top = el.offsetTop;
      next.set(key, top);
      if (!flip) continue;
      // "First" = where the element is *visually* right now, which for a glide
      // interrupted mid-flight means its committed layout top plus the running
      // animation's current translateY. Reading that (then cancelling the old
      // glide) lets the new one start from the live position — no jump on fast
      // typing.
      const m = new DOMMatrixReadOnly(getComputedStyle(el).transform);
      flipAnims.current.get(key)?.cancel();
      flipAnims.current.delete(key);
      const prev = flipTops.current.get(key);
      if (prev === undefined) continue;
      const delta = prev - top + m.m42;
      if (Math.abs(delta) < 0.5) continue;
      const anim = el.animate(
        [{ transform: `translateY(${delta}px)` }, { transform: "translateY(0)" }],
        { duration: 180, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
      );
      flipAnims.current.set(key, anim);
      anim.onfinish = () => { if (flipAnims.current.get(key) === anim) flipAnims.current.delete(key); };
    }
    flipTops.current = next;
    // Drop ref callbacks for rows that are no longer mounted. Only worth doing
    // once the cache has clearly outgrown the list — pruning on every result set
    // would hand surviving rows fresh callback identities and defeat their memo.
    if (flipRefFns.current.size > results.length * 4 + 64) {
      for (const key of flipRefFns.current.keys()) {
        if (!flipEls.current.has(key)) flipRefFns.current.delete(key);
      }
    }
  }, [results]);

  // Geometry of the sliding-selection layer: tracks the selected row's box so a
  // single highlight element can glide/resize to it. `snap` disables the glide
  // for moves that scrolled the list (see below). Labels paint above the bar
  // (CSS) so it slides behind them. null = hidden.
  const [indicator, setIndicator] = useState<{ top: number; height: number; snap: boolean } | null>(null);
  useLayoutEffect(() => {
    // The selected nodes come out of the flip registry rather than refs assigned
    // during render — that assignment is what forced every row to re-render to
    // stay correct. `labels` is the same first-in-group test the rows render from.
    const selectedResult = results[selectedIndex];
    const el = selectedResult ? flipEls.current.get(`r:${selectedResult.id}`) : undefined;
    const labelEl = selectedResult && labels[selectedIndex] !== null
      ? flipEls.current.get(`g:${selectedResult.id}`)
      : undefined;
    const col = colRef.current;
    if (!el || !col) { setIndicator(null); return; }
    // Keep the selection in view, here (pre-paint) so we can tell if it scrolled.
    // Row first so it's always visible, then the group label (if first-in-group)
    // so its header isn't clipped above the fold.
    const before = col.scrollTop;
    // Scroll using layout offsets, not scrollIntoView: during a FLIP glide the
    // selected row carries a transform, and scrollIntoView reads the transformed
    // (moving) box — scrolling to a mid-animation target makes the list lurch.
    // offsetTop is transform-independent. Anchor the top at the group label when
    // first-in-group so the header isn't clipped above the fold.
    const anchorTop = (labelEl ?? el).offsetTop;
    const bottom = el.offsetTop + el.offsetHeight;
    if (anchorTop < col.scrollTop) col.scrollTop = anchorTop;
    else if (bottom > col.scrollTop + col.clientHeight) col.scrollTop = bottom - col.clientHeight;
    // A scroll keeps the row at the same screen spot, but the bar's content-space
    // position jumps a full row — gliding that fights the instant scroll and
    // makes the bar lurch. Snap instead; glide only for in-view moves.
    const scrolled = col.scrollTop !== before;
    setIndicator({ top: el.offsetTop, height: el.offsetHeight, snap: scrolled });
  }, [selectedIndex, results]);

  return (
    <div className="results-col" ref={colRef} role="listbox">
      {indicator && results.length > 0 && (
        <div
          className="selection-bg"
          aria-hidden="true"
          style={{
            transform: `translateY(${indicator.top}px)`,
            height: indicator.height,
            transition: indicator.snap ? 'none' : undefined,
          } as CSSProperties}
        />
      )}
      {active && results.length === 0 && !searching && emptyReady && pending.length === 0 && (
        <div className="results-empty">{emptyLabel}</div>
      )}
      {results.map((result, i) => (
        <ResultRow
          key={result.id}
          result={result}
          index={i}
          isSelected={i === selectedIndex}
          label={labels[i]}
          shortcut={shortcuts.get(result.id) ?? 0}
          rowRef={flipRef(`r:${result.id}`)}
          labelRef={labels[i] !== null ? flipRef(`g:${result.id}`) : undefined}
          onSelect={handleSelect}
          onLaunch={handleLaunch}
        />
      ))}
      {active && pending.map(name => (
        <div className="result-pending" key={`pending:${name}`}>
          <span className="result-pending-spinner" aria-hidden="true" />
          <span className="result-pending-name">{name}</span>
          <span className="result-pending-label">searching…</span>
        </div>
      ))}
    </div>
  );
}
