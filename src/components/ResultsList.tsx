import { Fragment, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { SearchResult } from "../types";
import { groupLabel, formatBytes, shortenPath } from "../utils";
import ResultIcon from "./ResultIcon";

interface Props {
  results: SearchResult[];
  selectedIndex: number;
  /** Whether the user has entered a meaningful search term (drives the empty state). */
  active: boolean;
  searching?: boolean;
  onSelect: (index: number) => void;
  onLaunch: (result?: SearchResult) => void;
  launchableResults: SearchResult[];
  /** Per-result dominant icon colour for the accent-bleed effect. */
  accents: Map<string, string>;
  /** Empty-state text when a search resolves with no results. */
  emptyLabel?: string;
  /** Gate for showing the empty state — held false until the empty verdict has
   *  settled, so a dead prefix mid-typing stays blank instead of flashing. */
  emptyReady?: boolean;
  /** Names of extensions whose async query is still running - rendered as
   *  slim loading rows below the results. */
  pending?: string[];
}

export default function ResultsList({ results, selectedIndex, active, searching, onSelect, onLaunch, launchableResults, accents, emptyLabel = "No results", emptyReady = true, pending = [] }: Props) {
  const selectedRef = useRef<HTMLDivElement>(null);
  const selectedLabelRef = useRef<HTMLDivElement>(null);
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

  const flipRef = (key: string) => (el: HTMLElement | null) => {
    if (el) flipEls.current.set(key, el);
    else flipEls.current.delete(key);
  };

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
  }, [results]);

  // Geometry of the sliding-selection layer: tracks the selected row's box so a
  // single highlight element can glide/resize to it. `snap` disables the glide
  // for moves that scrolled the list (see below). Labels paint above the bar
  // (CSS) so it slides behind them. null = hidden.
  const [indicator, setIndicator] = useState<{ top: number; height: number; snap: boolean } | null>(null);
  useLayoutEffect(() => {
    const el = selectedRef.current;
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
    const anchorTop = (selectedLabelRef.current ?? el).offsetTop;
    const bottom = el.offsetTop + el.offsetHeight;
    if (anchorTop < col.scrollTop) col.scrollTop = anchorTop;
    else if (bottom > col.scrollTop + col.clientHeight) col.scrollTop = bottom - col.clientHeight;
    // A scroll keeps the row at the same screen spot, but the bar's content-space
    // position jumps a full row — gliding that fights the instant scroll and
    // makes the bar lurch. Snap instead; glide only for in-view moves.
    const scrolled = col.scrollTop !== before;
    setIndicator({ top: el.offsetTop, height: el.offsetHeight, snap: scrolled });
  }, [selectedIndex, results]);

  const selectedResult = results[selectedIndex];
  const selectedAccent = selectedResult ? accents.get(selectedResult.id) : undefined;

  return (
    <div className="results-col" ref={colRef} role="listbox">
      {indicator && results.length > 0 && (
        <div
          className={`selection-bg${selectedAccent ? " has-accent" : ""}`}
          aria-hidden="true"
          style={{
            transform: `translateY(${indicator.top}px)`,
            height: indicator.height,
            '--row-accent': selectedAccent,
            transition: indicator.snap ? 'none' : undefined,
          } as CSSProperties}
        />
      )}
      {active && results.length === 0 && !searching && emptyReady && pending.length === 0 && (
        <div className="results-empty">{emptyLabel}</div>
      )}
      {results.map((result, i) => {
        const label = groupLabel(result.kind);
        const prevLabel = i > 0 ? groupLabel(results[i - 1].kind) : null;
        const showLabel = label !== null && label !== prevLabel;
        const shortcutIdx = launchableResults.indexOf(result);
        const showShortcut = shortcutIdx >= 0 && shortcutIdx < 9;
        return (
          <Fragment key={result.id}>
            {showLabel && (
              <div
                ref={el => {
                  flipRef(`g:${result.id}`)(el);
                  if (i === selectedIndex) selectedLabelRef.current = el;
                }}
                className={`result-group-label${i === 0 ? " first" : ""}`}
              >
                <span>{label}</span>
              </div>
            )}
            <div
              ref={el => {
                flipRef(`r:${result.id}`)(el);
                if (i === selectedIndex) selectedRef.current = el;
              }}
              className={`result-row${i === selectedIndex ? " selected" : ""}${accents.has(result.id) ? " has-accent" : ""}`}
              data-kind={result.kind}
              style={{ '--row-i': i, '--row-accent': accents.get(result.id) } as CSSProperties}
              role="option"
              aria-selected={i === selectedIndex}
              onClick={() => {
                onSelect(i);
                onLaunch(result);
              }}
            >
              <ResultIcon icon_path={result.icon_path} iconDataUri={result.icon_data_uri} glyph={result.command?.glyph} title={result.title} kind={result.kind} />
              <div className="result-text">
                <div className="result-title">{result.title}</div>
                {result.subtitle && <div className="result-subtitle">{shortenPath(result.subtitle)}</div>}
              </div>
              <div className="result-meta">
                {result.ext?.badge
                  ? <span className="result-badge">{result.ext.badge}</span>
                  : result.kind === "file" && result.file_size != null
                    ? formatBytes(result.file_size)
                    : ""}
              </div>
              <div className="result-shortcut" style={!showShortcut ? { visibility: 'hidden' } : undefined}>
                {showShortcut ? shortcutIdx + 1 : ""}
              </div>
            </div>
          </Fragment>
        );
      })}
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
