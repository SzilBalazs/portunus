import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import type { OfficeShape } from "../../types";
import { buildOfficeSrcdoc, clampOfficeZoom, randomToken, OFFICE_ZOOM_STEP } from "../../srcdoc";
import { hostFocus } from "../../focus";
import { selection } from "../../selection/controller";
import OfficeFrame from "./OfficeFrame";
import type { OfficeFrameHandle } from "./OfficeFrame";
import type { FrameSelState, HostMessage } from "./protocol";
import { officeScroll } from "./scrollRegistry";
import { useOfficeHtml } from "./useOfficeHtml";

/**
 * Extensions the Rust HTML renderer handles today. Everything else keeps going
 * through the markdown / grid path (`fallback`); later backend stages move
 * `docx`, `pptx` and the ODF formats over by adding them here.
 */
const HTML_RENDERED_EXTS = new Set(["xlsx"]);

/** How long a re-render may take before the skeleton covers the stale document. */
const SKELETON_DELAY_MS = 140;

/** How long the zoom readout stays up after the last step. */
const ZOOM_BADGE_MS = 900;

// ── theme epoch ──────────────────────────────────────────────────────────────

// buildOfficeSrcdoc bakes the theme's custom-property *values* into the document
// (an iframe cannot inherit them), and the srcdoc is memoized - so a theme change
// is invisible until the document is rebuilt. This store bumps on the two events
// that change those values and folds into the srcdoc memo's deps, reloading both
// buffers exactly once per theme change.
let themeEpoch = 0;
const themeListeners = new Set<() => void>();
function bumpTheme(delayMs: number) {
  // Deferred, because both events are *notifications*: App.tsx's own listener is
  // what writes the new values onto the host root (and for theme-css-changed it
  // first re-fetches the injected Matugen CSS). Reading getComputedStyle in the
  // same tick would bake the outgoing palette.
  setTimeout(() => {
    themeEpoch++;
    themeListeners.forEach(l => l());
  }, delayMs);
}
void listen("appearance-changed", () => bumpTheme(0));
void listen("theme-css-changed", () => bumpTheme(150));
const subscribeTheme = (cb: () => void) => {
  themeListeners.add(cb);
  return () => void themeListeners.delete(cb);
};

/** Keys the frame's selection engine needs forwarded: it is never focused, so it
 *  cannot receive them itself (see the focus-custody note in `officeBootstrap`). */
const SEL_KEYS = new Set(["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"]);

// ── dispatcher ───────────────────────────────────────────────────────────────

interface Props {
  path: string;
  filename: string;
  shape: OfficeShape;
  terms: string[];
  highlight: boolean;
  /** The pre-existing markdown / grid renderer, for formats the HTML renderer
   *  does not cover yet. */
  fallback: ReactNode;
}

/**
 * Routes an office file to the rendered-HTML frame or to the legacy renderer.
 *
 * The gate is the file extension, not the shape alone: `ods` is a sheet too, but
 * the backend has no ODF HTML renderer yet and would answer with an error.
 */
export default function OfficePreview({ path, filename, shape, terms, highlight, fallback }: Props) {
  const ext = filename.split(".").pop()?.toLowerCase() ?? "";
  if (shape !== "sheet" || !HTML_RENDERED_EXTS.has(ext)) return <>{fallback}</>;
  return <OfficeHtmlPreview path={path} filename={filename} terms={terms} highlight={highlight} />;
}

// ── rendered-HTML preview ────────────────────────────────────────────────────

// The variant the frame is scaffolded for comes from the *rendered* document
// (`doc.shape`), not from the extension - the renderer is the authority on what
// it produced.
function OfficeHtmlPreview({
  path,
  filename,
  terms,
  highlight,
}: Omit<Props, "fallback" | "shape">) {
  const epoch = useSyncExternalStore(subscribeTheme, () => themeEpoch);
  // `null` means "whichever section the backend considers the default" (for a
  // workbook, the first non-hidden sheet). The tab strip sets it to an index.
  const [section, setSection] = useState<number | null>(null);
  // Reset on a file change. PreviewPanel keys by `result.kind` and every file
  // result shares one kind, so this component instance is reused across results -
  // without this, sheet 3 of the last workbook would be requested for the next
  // file, which very likely does not have one. Assigned during render, not in an
  // effect, for the reason spelled out in useOfficeHtml: an effect commits the
  // new path paired with the old section first.
  const pathRef = useRef(path);
  if (pathRef.current !== path) {
    pathRef.current = path;
    setSection(null);
  }
  const { doc, error, loading } = useOfficeHtml(path, section, terms);

  const frameRef = useRef<OfficeFrameHandle>(null);
  // Read inside the srcdoc memo without joining its deps: the highlight state is
  // baked in for the first paint, then driven by message (see below) so Ctrl+H
  // never reloads the document.
  const hlRef = useRef(highlight);
  hlRef.current = highlight;

  // Zoom lives in a ref for the same reason: it is baked into the next document
  // the memo builds (so a sheet switch does not snap back to 100%) but changing
  // it must never rebuild the current one - that is a message. `zoomLabel` is the
  // transient readout and is the only part React re-renders on.
  const zoomRef = useRef(1);
  const [zoomLabel, setZoomLabel] = useState<number | null>(null);
  const badgeTimer = useRef<number | undefined>(undefined);
  const showZoomBadge = useCallback((z: number) => {
    setZoomLabel(z);
    window.clearTimeout(badgeTimer.current);
    badgeTimer.current = window.setTimeout(() => setZoomLabel(null), ZOOM_BADGE_MS);
  }, []);
  useEffect(() => () => window.clearTimeout(badgeTimer.current), []);

  const built = useMemo(() => {
    if (!doc) return null;
    const token = randomToken();
    return {
      token,
      srcdoc: buildOfficeSrcdoc(doc.html, doc.shape, {
        token,
        bestMarkId: doc.bestMarkId,
        hlOff: !hlRef.current,
        natural: doc.natural,
        page: doc.page,
        zoom: zoomRef.current,
      }),
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [doc, epoch]);

  // Bumped on every `ready`, so the highlight state is re-asserted on the buffer
  // that just came up (it was built with whatever was current at build time, which
  // may already be stale by the time it finishes loading).
  const [readyTick, setReadyTick] = useState(0);
  const paintedRef = useRef(false);

  useEffect(() => {
    frameRef.current?.send({ type: "hl", on: highlight });
  }, [highlight, readyTick]);

  // Let QuickLook scroll us: an office document's scroller is in another document,
  // so there is no DOM viewport for it to find. Stable identity, because the
  // registry arbitrates side-panel vs Quicklook by which handler is on top.
  const send = useCallback((msg: HostMessage) => frameRef.current?.send(msg) ?? false, []);
  useEffect(() => officeScroll.register(send), [send]);

  // ── selection ──────────────────────────────────────────────────────────────
  // The frame runs its own copy of the selection engine (frameSelection.ts) and
  // reports the result; the host adopts it so the copy and search-selection
  // chords, the footer bar and the popover behave as they do for any other
  // preview. See `adoptExternal` in selection/controller.ts.
  const wrapRef = useRef<HTMLDivElement>(null);
  const selRef = useRef<FrameSelState | null>(null);
  // Bumped only when the selected *text* changes, so the popover survives the
  // stream of anchor restatements a scroll produces (see ExternalPopover.key).
  const selEpoch = useRef(0);

  const onSel = useCallback((s: FrameSelState) => {
    if (selRef.current?.text !== s.text) selEpoch.current++;
    selRef.current = s;
    // The popover is host DOM, so the frame's viewport coordinates have to be
    // mapped into the wrap's. The scale is derived from the frame's painted box
    // against the viewport width it reported, which stays right however the
    // launcher's UI zoom and the reader's zoom happen to be plumbed.
    let popover = null;
    const wrap = wrapRef.current;
    const fr = frameRef.current?.rect();
    const show = s.text !== "" && !s.dragging && !s.keyboard && s.anchor;
    if (show && wrap && fr && s.vw > 0 && s.vh > 0) {
      const wr = wrap.getBoundingClientRect();
      const kx = fr.width / s.vw;
      const ky = fr.height / s.vh;
      const [ax, ay, aw, ah] = s.anchor!;
      popover = {
        host: wrap,
        key: selEpoch.current,
        anchor: {
          x: fr.left - wr.left + ax * kx,
          y: fr.top - wr.top + ay * ky,
          w: aw * kx,
          h: ah * ky,
        },
        viewport: {
          top: fr.top - wr.top,
          bottom: fr.bottom - wr.top,
          left: fr.left - wr.left,
          right: fr.right - wr.left,
        },
      };
    }
    selection.adoptExternal({
      text: s.text,
      keyboard: s.keyboard,
      clear: () => void send({ type: "selClear" }),
      popover,
    });
  }, [send]);

  // A new document has nothing selected in it, and the frame reports nothing on
  // load - so the release has to happen here. Unmount included: the preview can
  // go away with a selection live in it.
  // Guarded on actually owning one, so the side panel unmounting under an open
  // Quicklook cannot drop the overlay's selection.
  const releaseSel = useCallback(() => {
    const s = selRef.current;
    selRef.current = null;
    if (s && (s.text !== "" || s.keyboard)) selection.adoptExternal(null);
  }, []);
  useEffect(() => releaseSel, [releaseSel]);

  const setZoom = useCallback((z: number) => {
    const next = clampOfficeZoom(z);
    zoomRef.current = next;
    send({ type: "zoom", factor: next });
    showZoomBadge(next);
  }, [send, showZoomBadge]);

  // ── sections ───────────────────────────────────────────────────────────────
  // The section list comes from the rendered document, so while a switch is in
  // flight it is the *previous* document's - which is the same file, so the strip
  // stays correct. The highlight follows the requested index rather than
  // `doc.section`, so a click responds on the click and not one render later.
  const sections = doc?.sections ?? [];
  const count = sections.length;
  const active = Math.min(section ?? doc?.section ?? 0, Math.max(count - 1, 0));
  const gotoSection = useCallback(
    (i: number) => setSection(Math.min(Math.max(i, 0), Math.max(count - 1, 0))),
    [count],
  );

  // Keep the selected tab reachable in a workbook with more sheets than fit.
  const activeTabRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    activeTabRef.current?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [active]);

  // Ctrl +/-/0 zooms; Ctrl+arrows and Ctrl+Home/End move between sections.
  //
  // The frame runs the same zoom handler for the case where focus slipped inside
  // it, and ctrl+wheel is handled there outright (the wheel event is delivered to
  // the frame, never to the host) - both report back through `onZoomed`, which is
  // what keeps this ref in step.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.altKey || e.metaKey || !document.hasFocus()) return;
      // Only the topmost mount acts: the side panel stays mounted underneath an
      // open Quicklook, and both install this listener.
      if (!officeScroll.isTop(send)) return;
      // Keyboard caret mode owns the movement keys outright - including Ctrl+arrows,
      // which mean word-move there, exactly as they do in the host engine, and so
      // never reach the section flip below. Escape is deliberately not forwarded:
      // App.tsx already dismisses a live selection before anything else, and for an
      // adopted one that routes back into the frame as `selClear`.
      if (selRef.current?.keyboard && SEL_KEYS.has(e.key)) {
        send({ type: "selKey", key: e.key, shift: e.shiftKey, ctrl: e.ctrlKey });
        e.preventDefault();
        e.stopPropagation();
        return;
      }
      if (!e.ctrlKey) return;
      // In keyboard select mode Ctrl+arrows mean word-move, as they do for the PDF
      // reader's page flip.
      const navigable = count > 1 && !selection.isKeyboardMode();
      if (e.key === "=" || e.key === "+") setZoom(zoomRef.current * OFFICE_ZOOM_STEP);
      else if (e.key === "-" || e.key === "_") setZoom(zoomRef.current / OFFICE_ZOOM_STEP);
      else if (e.key === "0") setZoom(1);
      else if (!navigable) return;
      else if (e.key === "ArrowLeft") gotoSection(active - 1);
      else if (e.key === "ArrowRight") gotoSection(active + 1);
      else if (e.key === "Home") gotoSection(0);
      else if (e.key === "End") gotoSection(count - 1);
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [send, setZoom, gotoSection, active, count]);

  // Skeleton is an *overlay* on the still-painting stale frame, never a reason to
  // blank it (`srcDoc=null` would be two extra navigations, i.e. two flashes).
  // Delayed, so a cached or fast render shows no loading state at all - except on
  // the first ever mount, which has nothing to hold and would otherwise be a void.
  const [showSkeleton, setShowSkeleton] = useState(true);
  useEffect(() => {
    // Nothing has ever painted here - even a cache hit still has to navigate a
    // frame, so hold the skeleton until `ready` rather than flash the empty wrap.
    if (!paintedRef.current) {
      setShowSkeleton(true);
      return;
    }
    // A document is already on screen. Drop the skeleton as soon as the render
    // resolves: the stale frame keeps painting until the back buffer flips in, so
    // there is nothing left to mask.
    if (!loading) {
      setShowSkeleton(false);
      return;
    }
    const t = setTimeout(() => setShowSkeleton(true), SKELETON_DELAY_MS);
    return () => clearTimeout(t);
  }, [loading]);

  const notes = doc?.notes ?? [];
  const footer =
    doc && (notes.length > 0 || doc.truncated) ? (
      <div className="office-notes">
        {doc.truncated && (
          <div className="office-note">This document is larger than the preview shows.</div>
        )}
        {notes.map((n, i) => (
          <div className="office-note" key={i}>{n}</div>
        ))}
      </div>
    ) : null;

  // Nothing rendered and nothing to hold: show the failure instead of an empty
  // frame stretched over the panel.
  if (error && !doc) {
    return (
      <div className="office-preview">
        <div className="office-msg">{error}</div>
      </div>
    );
  }

  return (
    <div className="office-preview">
      <div className="office-frame-wrap" ref={wrapRef}>
        {built && (
          <OfficeFrame
            ref={frameRef}
            srcdoc={built.srcdoc}
            token={built.token}
            title={`${filename} preview`}
            onReady={() => {
              paintedRef.current = true;
              setShowSkeleton(false);
              setReadyTick(t => t + 1);
              releaseSel();
            }}
            onSel={onSel}
            onZoomed={z => {
              if (z === zoomRef.current) return;
              zoomRef.current = z;
              showZoomBadge(z);
            }}
            onRefocus={() => hostFocus.restore()}
          />
        )}
        {zoomLabel !== null && (
          <div className="office-zoom-badge">{Math.round(zoomLabel * 100)}%</div>
        )}
        {showSkeleton && <div className="office-skeleton" />}
      </div>
      {count > 1 && (
        // Below the document, where a spreadsheet's sheet tabs live. `tabIndex={-1}`
        // and no focus outline because the launcher keeps focus on the search input
        // - cancelling the mousedown does not cancel the click, so these still work.
        <div className="office-tabs" role="tablist" aria-label="Sheets">
          {sections.map((name, i) => (
            <button
              key={i}
              ref={i === active ? activeTabRef : undefined}
              className={`office-tab${i === active ? " is-active" : ""}`}
              role="tab"
              aria-selected={i === active}
              tabIndex={-1}
              title={name || `Sheet ${i + 1}`}
              onClick={() => gotoSection(i)}
            >
              {name || `Sheet ${i + 1}`}
            </button>
          ))}
        </div>
      )}
      {footer}
    </div>
  );
}
