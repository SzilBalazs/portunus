import { useState, useEffect, useLayoutEffect, useRef, useMemo, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";
import { Config, SearchResult, SearchResponse, StreamPayload, ClipboardCapabilities, ExtAction } from "./types";
import ClipboardMode from "./components/clipboard/ClipboardMode";
import { applyTheme, injectMatugenTheme } from "./theme";
import ResultsList from "./components/ResultsList";
import PreviewPanel from "./components/PreviewPanel";
import QuickLook from "./components/QuickLook";
import ActionPicker from "./components/ActionPicker";
import { useExtensionMeta, matchTrigger } from "./extensions/meta";
import { deriveContentTerms } from "./highlight";
import FooterHints from "./components/FooterHints";
import { pdfView } from "./components/FilePreview";
import { isPreviewable } from "./utils";
import { ColoredIconsContext } from "./coloredIcons";
import { dispatchLaunch, dispatchKeyDown, type LaunchContext } from "./providers/registry";
import { useTauriListener } from "./hooks/useTauriListener";
import { useIconAccents } from "./hooks/useIconAccents";
import OnboardingWizard from "./components/onboarding/OnboardingWizard";
import "./providers";
import "./App.css";
import "./themes.css";

const NON_INDEXABLE_KINDS = new Set(['calc', 'dict', 'dict-hint', 'content-hint', 'content-disabled', 'search-error']);

// Greyed-out completion shown after a partial command word (e.g. "def" -> "ine").
// Tab accepts it. Returns the suffix to append, or null when nothing completes.
function ghostFor(q: string): string | null {
  if (q.length < 2 || q.includes(' ')) return null;
  if ('define'.startsWith(q) && q.length < 6) return 'define'.slice(q.length);
  if ('dict'.startsWith(q) && q.length < 4 && !'define'.startsWith(q)) return 'dict'.slice(q.length);
  if ('clipboard'.startsWith(q) && q.length >= 4 && q.length < 9) return 'clipboard'.slice(q.length);
  return null;
}

export default function App() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  // Async-tier state: batches streamed per extension for the current query,
  // and the set of extensions still working. Both keyed off queryIdRef so
  // stale events from a cancelled query can never render.
  const [streamed, setStreamed] = useState<Map<string, SearchResult[]>>(new Map());
  const [pendingExts, setPendingExts] = useState<Set<string>>(new Set());
  // Monotonic per-dispatch id correlating a `search` invoke with its
  // `search-stream` events. Seeded with wall-clock so a webview reload can't
  // reuse ids the backend has already seen (its reorder guard is a fetch_max).
  const queryIdRef = useRef(Date.now());
  // Extensions whose async query already finished for the current dispatch.
  // The `search` response races the first `search-stream` done events, so its
  // pendingExts snapshot filters against this to avoid a stuck spinner.
  const doneExtsRef = useRef<Set<string>>(new Set());
  // True between scheduling a search and its results arriving, so the results
  // list can distinguish "still loading" from a genuine zero-result query.
  const [searching, setSearching] = useState(false);
  // True only once a non-empty query has actually resolved with zero results.
  // Gates the content-search hint so it never flashes on the first keystroke
  // (stale empty results + pending debounce) yet stays mounted across re-searches.
  const [resolvedEmpty, setResolvedEmpty] = useState(false);
  // True when the last search invoke rejected, so the list can show an error row
  // instead of masquerading the failure as a genuine zero-result query.
  const [searchError, setSearchError] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(0);
  // The pinned result shown in Quicklook (null = closed). Pinning the result -
  // rather than tracking a boolean + selectedIndex - keeps the overlay on the
  // same file even if background events reorder/extend the results underneath.
  const [quickResult, setQuickResult] = useState<SearchResult | null>(null);
  // Matched-term highlighting in the PDF preview overlay; Ctrl+H toggles it.
  const [highlight, setHighlight] = useState(true);
  const [loading, setLoading] = useState(true);
  const [version, setVersion] = useState("");
  const [indexingProgress, setIndexingProgress] = useState<{ indexed: number; total: number } | null>(null);
  const [contentEnabled, setContentEnabled] = useState(true);
  const [coloredIcons, setColoredIcons] = useState(true);
  // Full config kept in a ref so the (effect-bound) keydown handler reads the
  // current value without re-subscribing. Refreshed on mount/show/invalidation.
  const configRef = useRef<Config | null>(null);
  const [onboardConfig, setOnboardConfig] = useState<Config | null>(null);
  const [showOnboarding, setShowOnboarding] = useState(false);
  const showOnboardingRef = useRef(false);
  useEffect(() => { showOnboardingRef.current = showOnboarding; }, [showOnboarding]);
  const inputRef = useRef<HTMLInputElement>(null);
  const mirrorRef = useRef<HTMLSpanElement>(null);
  const [inputWidth, setInputWidth] = useState(0);
  const queryRef = useRef(query);
  const ghostRef = useRef<string | null>(null);
  // Whether the launcher window currently holds focus. Guards the global key
  // handler so an always-on-top, unfocused window doesn't eat arrow keys etc.
  const focusedRef = useRef(true);
  // Mirror of "Quicklook open" for use inside event-listener closures.
  const quicklookRef = useRef(false);
  // The extension result pinned by the action picker (null = closed). Pins the
  // result like Quicklook so background reorders can't retarget the actions.
  const [actionResult, setActionResult] = useState<SearchResult | null>(null);
  const actionResultRef = useRef(false);
  // Transient toast from an extension's ShowToast activate effect.
  const [toast, setToast] = useState<string | null>(null);

  // Dedicated clipboard-history browser. While active the launcher's search effect
  // and global key handler stand down; ClipboardMode owns input handling.
  const [clipboardMode, setClipboardMode] = useState(false);
  const clipboardModeRef = useRef(false);
  useEffect(() => { clipboardModeRef.current = clipboardMode; }, [clipboardMode]);
  const [clipCaps, setClipCaps] = useState<ClipboardCapabilities>({ smart_paste: false });
  const capsFetched = useRef(false);
  // Whether the browser was opened via `portunus --clipboard` (so Esc hides the
  // window) vs typed into the launcher (so Esc returns to the launcher).
  const clipFromFlag = useRef(false);

  const enterClipboardMode = (seed = "", fromFlag = false) => {
    if (!capsFetched.current) {
      capsFetched.current = true;
      invoke<ClipboardCapabilities>("clipboard_capabilities").then(setClipCaps).catch(() => {});
    }
    clipFromFlag.current = fromFlag;
    // Clipboard overrides content mode so only one chip is ever active.
    setContentMode(false);
    setClipboardMode(true);
    setQuery(seed);
    setResults([]);
    inputRef.current?.focus();
  };

  const exitClipboardMode = () => {
    if (clipFromFlag.current) {
      setClipboardMode(false);
      setQuery("");
      setResults([]);
      invoke("hide_window");
      return;
    }
    setClipboardMode(false);
    setQuery("");
    setResults([]);
    inputRef.current?.focus();
  };

  // Dedicated full-text "Contents" mode. Unlike clipboard it reuses the normal
  // results list/preview - only the search routing, chrome, and a few keys change.
  // Tab toggles it; Backspace on an empty query drops the chip.
  const [contentMode, setContentMode] = useState(false);
  const contentModeRef = useRef(false);
  useEffect(() => { contentModeRef.current = contentMode; }, [contentMode]);

  // Tab toggles content mode, keeping the typed query so a search flips between
  // matching names and matching file contents.
  const enterContentMode = (seed?: string) => {
    setContentMode(true);
    if (seed !== undefined) setQuery(seed);
    setResults([]);
    setResolvedEmpty(false);
    inputRef.current?.focus();
  };

  const exitContentMode = () => {
    setContentMode(false);
    setResults([]);
    setResolvedEmpty(false);
    inputRef.current?.focus();
  };

  useEffect(() => { queryRef.current = query; }, [query]);
  useEffect(() => { getVersion().then(setVersion); }, []);

  // Load config on mount to apply theme and read content.enabled; re-apply theme on settings changes.
  useEffect(() => {
    invoke<Config>("get_config").then(cfg => {
      applyTheme(cfg.appearance);
      setContentEnabled(cfg.content.enabled);
      setColoredIcons(cfg.files.colored_icons);
      configRef.current = cfg;
      setOnboardConfig(cfg);
      if (!cfg.general.onboarding_completed) setShowOnboarding(true);
    });
    const unlisteners: Array<() => void> = [];
    listen<Config["appearance"]>("appearance-changed", event => {
      applyTheme(event.payload);
    }).then(fn => { unlisteners.push(fn); });
    // matugen post_hook (portunus --reload-theme) → re-fetch + re-inject the
    // external CSS, then re-apply the current theme so live edits show instantly.
    listen("theme-css-changed", () => {
      injectMatugenTheme().then(() => {
        if (configRef.current) applyTheme(configRef.current.appearance);
      });
    }).then(fn => { unlisteners.push(fn); });
    return () => { unlisteners.forEach(fn => fn()); };
  }, []);

  useLayoutEffect(() => {
    setInputWidth(mirrorRef.current?.offsetWidth ?? 0);
  }, [query]);

  useEffect(() => {
    let done = false;
    const markReady = () => { if (!done) { done = true; setLoading(false); } };
    const promise = listen("apps-ready", markReady);
    invoke<boolean>("is_apps_ready").then(ready => { if (ready) markReady(); });
    return () => { done = true; promise.then(ul => ul()); };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    let doneTimer: ReturnType<typeof setTimeout> | undefined;
    listen<{ indexed: number; total: number }>("content-index-progress", event => {
      const p = event.payload;
      setIndexingProgress(p);
      clearTimeout(doneTimer);
      if (p.indexed >= p.total && p.total > 0) {
        doneTimer = setTimeout(() => setIndexingProgress(null), 150);
      }
    }).then(fn => {
      if (active) unlisten = fn; else fn();
    });
    return () => { active = false; clearTimeout(doneTimer); unlisten?.(); };
  }, []);

  useTauriListener("window-show", () => {
    focusedRef.current = true;
    // A plain `--show` always opens the clean launcher, never a stale clipboard
    // session. (`--clipboard` uses window-show-query, which re-enters the mode.)
    setClipboardMode(false);
    setContentMode(false);
    setActionResult(null);
    setQuery("");
    setResults([]);
    setStreamed(new Map());
    setPendingExts(new Set());
    inputRef.current?.focus();
    invoke<Config>("get_config").then(cfg => { setContentEnabled(cfg.content.enabled); setColoredIcons(cfg.files.colored_icons); configRef.current = cfg; });
  });

  useEffect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    win.onFocusChanged(({ payload: focused }) => {
      focusedRef.current = focused;
      // Don't steal focus back into the input while Quicklook is open (modal).
      if (focused && !quicklookRef.current) inputRef.current?.focus();
    }).then(fn => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);

  useTauriListener<string>("window-show-query", payload => {
    // `portunus --clipboard` sends "clipboard " - open the dedicated browser
    // instead of pre-filling the launcher query.
    if (payload.trim() === "clipboard") { enterClipboardMode("", true); return; }
    setQuery(payload);
    // Don't manually clear results - the query useEffect will re-search immediately.
    inputRef.current?.focus();
  });

  // Typed entry: "clip…" + space opens the browser, seeding the filter with the
  // remainder (e.g. "clipboard foo" → mode filtered by "foo"). Same prefix the
  // backend provider triggers on; mirrors the inline provider's old behavior.
  useEffect(() => {
    if (clipboardMode || contentMode) return;
    const m = /^(clip|clipb|clipbo|clipboa|clipboar|clipboard)\s+(.*)$/i.exec(query);
    if (m) enterClipboardMode(m[2], false);
  }, [query, clipboardMode, contentMode]);

  // Refs mirroring the async-tier state for use inside the (deliberately
  // dependency-light) search effect below.
  const streamedRef = useRef(false);
  const pendingRef = useRef(false);
  useEffect(() => { streamedRef.current = streamed.size > 0; }, [streamed]);
  useEffect(() => { pendingRef.current = pendingExts.size > 0; }, [pendingExts]);

  useEffect(() => {
    if (clipboardMode) { setSearching(false); return; }
    const trimmed = query.trim();
    // Content search needs >= 2 chars (matches the backend guard); below that
    // there's nothing to run, so clear and show the prompt empty state.
    if (!trimmed || (contentMode && trimmed.length < 2)) {
      setResults([]); setSearching(false); setResolvedEmpty(false);
      // Nothing to search = nothing the async tier should keep working on.
      if (streamedRef.current || pendingRef.current) {
        setStreamed(new Map());
        setPendingExts(new Set());
        invoke("cancel_search").catch(() => {});
      }
      return;
    }
    let cancelled = false;
    let graceTimer: ReturnType<typeof setTimeout> | undefined;
    setSearching(true);
    setSearchError(false);
    // Clear any prior empty verdict so a resumed query doesn't keep the old
    // "no results" state visible while the new search is in flight.
    setResolvedEmpty(false);
    // Content search is the heavy path (FTS + snippet + per-keystroke preview
    // match-page), so coalesce keystrokes harder there. Name/app search is cheap
    // and stays near-instant.
    const debounceMs = contentMode ? 90 : 10;
    // A dead prefix mid-word (or the porter stem gap prefix matching can't cover)
    // resolves to zero for a beat before the next keystroke matches. Hold the
    // empty verdict behind this settle so the list stays neutral-blank instead of
    // flashing "no results" while the user is still typing.
    const EMPTY_SETTLE_MS = 220;
    const t = setTimeout(() => {
      if (contentMode) {
        invoke<SearchResult[]>("search_content", { query }).then(r => {
          if (cancelled) return;
          setResults(r);
          setSearching(false);
          if (r.length === 0) {
            graceTimer = setTimeout(() => { if (!cancelled) setResolvedEmpty(true); }, EMPTY_SETTLE_MS);
          } else {
            setResolvedEmpty(false);
          }
        }).catch(e => {
          console.error(`[search] search_content failed:`, e);
          if (!cancelled) { setResults([]); setSearching(false); setResolvedEmpty(false); setSearchError(true); }
        });
        return;
      }
      const queryId = ++queryIdRef.current;
      // New dispatch: everything streamed belongs to an older query now.
      // Clear here, not in .then - the backend dispatches async workers
      // before the search response resolves, so a fast extension's first
      // batch (or done signal) can legitimately arrive ahead of it.
      setStreamed(new Map());
      doneExtsRef.current = new Set();
      invoke<SearchResponse>("search", { query, queryId }).then(resp => {
        if (cancelled || resp.query_id !== queryIdRef.current) return;
        setResults(resp.results);
        setPendingExts(new Set(
          resp.pending.map(p => p.name).filter(n => !doneExtsRef.current.has(n)),
        ));
        setSearching(false);
        // The empty verdict waits for the async tier: a pending slow
        // extension may still deliver results (see the verdict effect below).
        if (resp.results.length === 0 && resp.pending.length === 0) {
          graceTimer = setTimeout(() => { if (!cancelled) setResolvedEmpty(true); }, EMPTY_SETTLE_MS);
        } else {
          setResolvedEmpty(false);
        }
      }).catch(e => {
        // Surface the failure as an error row rather than an empty result, which
        // would wrongly offer the "search file contents" hint.
        console.error(`[search] search failed:`, e);
        if (!cancelled) { setResults([]); setSearching(false); setResolvedEmpty(false); setSearchError(true); }
      });
    }, debounceMs);
    return () => { cancelled = true; clearTimeout(t); if (graceTimer) clearTimeout(graceTimer); };
  }, [query, clipboardMode, contentMode]);

  // Streamed async-query batches: merge per extension, replacing by result id
  // (a later batch re-emitting an id updates it in place - the intended
  // "cached now, fresh later" pattern).
  useTauriListener<StreamPayload>("search-stream", p => {
    if (p.query_id !== queryIdRef.current) return;
    if (p.results.length > 0) {
      // Merge batches by id. The keystroke path clears `streamed` before
      // dispatch (see the search effect), so merging into an empty map is
      // identical to replacing. A requery (same query text, new query_id)
      // does NOT clear streamed, so merging keeps the extension's rows
      // visible and updates them in place instead of blanking the list -
      // which would collapse the row count and jump the selection.
      setStreamed(prev => {
        const next = new Map(prev);
        const byId = new Map((next.get(p.ext) ?? []).map(r => [r.id, r] as const));
        for (const r of p.results) byId.set(r.id, r);
        next.set(p.ext, [...byId.values()]);
        return next;
      });
    }
    if (p.done) {
      if (p.error) console.warn(`[extension] ${p.ext} query failed: ${p.error}`);
      // Record completion even if the search response (which populates
      // pendingExts) hasn't resolved yet - it filters against this set.
      doneExtsRef.current.add(p.ext);
      setPendingExts(prev => {
        if (!prev.has(p.ext)) return prev;
        const next = new Set(prev);
        next.delete(p.ext);
        return next;
      });
    }
  });

  // Deferred empty verdict for the async tier: sync results were empty, and
  // the last pending extension just finished without delivering anything.
  useEffect(() => {
    if (contentMode || clipboardMode || searching || searchError) return;
    if (!query.trim()) return;
    if (pendingExts.size > 0 || results.length > 0 || streamed.size > 0) return;
    const t = setTimeout(() => setResolvedEmpty(true), 220);
    return () => clearTimeout(t);
  }, [pendingExts, results, streamed, searching, searchError, query, contentMode, clipboardMode]);

  useEffect(() => {
    setSelectedIndex(0);
    selectedIdRef.current = null;
    userMovedRef.current = false;
  }, [query, contentMode]);

  // Sync base merged with streamed async batches: replace by id (newest wins),
  // stable-sort by score so late arrivals slot in without jitter on ties.
  const merged = useMemo<SearchResult[]>(() => {
    if (streamed.size === 0) return results;
    const byId = new Map(results.map(r => [r.id, r]));
    for (const batch of streamed.values()) {
      for (const r of batch) byId.set(r.id, r);
    }
    return [...byId.values()].sort((a, b) => b.score - a.score);
  }, [results, streamed]);

  const displayResults = useMemo<SearchResult[]>(() => {
    if (!query.trim()) return [];
    if (searchError) {
      return [{
        id: "search:error",
        title: "Search failed",
        subtitle: "Something went wrong - check the logs and try again",
        kind: "search-error",
        score: 0,
      }];
    }
    if (contentMode) {
      if (!contentEnabled) {
        return [{
          id: "content:disabled",
          title: "Content search is disabled",
          subtitle: "Open Settings → Content to enable",
          kind: "content-disabled",
          score: 0,
        }];
      }
      // Below the 2-char minimum there's nothing to search yet.
      if (query.trim().length < 2) return [];
      return results;
    }
    if (merged.length === 0) {
      // Suggest content search only once a search has actually resolved empty
      // (not on the first-keystroke debounce gap). Kept mounted across
      // re-searches so it doesn't unmount/remount and re-animate per keystroke.
      if (!resolvedEmpty) return [];
      return [{
        id: "content:hint",
        title: "Search file contents",
        subtitle: `Search for "${query.trim()}" inside files`,
        kind: "content-hint",
        score: 0,
      }];
    }
    return merged;
  }, [query, results, merged, contentEnabled, resolvedEmpty, contentMode, searchError]);

  // Cursor stability under streaming. `selectedIdRef` is the identity of the
  // highlighted result; it's written at every mutation site (nav handlers,
  // click-select, this effect) so it never lags behind selectedIndex - the
  // old split "write the id in a second effect keyed on displayResults" raced,
  // reading a stale index against a fresh list and briefly pinning the wrong id.
  //
  // Pinning (follow a result to its new slot when a late batch re-sorts) only
  // applies once the user has actually navigated. On the untouched default the
  // highlight tracks row 0, so a better late arrival landing on top takes the
  // highlight naturally instead of dragging it down to row 1 on its own.
  const selectedIdRef = useRef<string | null>(null);
  const userMovedRef = useRef(false);
  useEffect(() => {
    setSelectedIndex(i => {
      const id = selectedIdRef.current;
      if (userMovedRef.current && id) {
        const idx = displayResults.findIndex(r => r.id === id);
        if (idx >= 0) {
          selectedIdRef.current = displayResults[idx].id;
          return idx;
        }
        // Pinned row is temporarily absent - e.g. a requery replaced an
        // extension's rows before re-delivering them. Do NOT overwrite the
        // pin here: keep it so the highlight re-selects the row when it
        // returns instead of getting stranded at the clamped index (which
        // would look like the selection jumping to the top).
        return i >= displayResults.length ? Math.max(0, displayResults.length - 1) : i;
      }
      const next = i >= displayResults.length ? 0 : i;
      selectedIdRef.current = displayResults[next]?.id ?? null;
      return next;
    });
  }, [displayResults]);

  const requery = () => {
    const q = queryRef.current;
    if (!q.trim()) return;
    if (contentModeRef.current) {
      invoke<SearchResult[]>("search_content", { query: q }).then(setResults)
        .catch(e => console.error(`[search] search_content requery failed:`, e));
      return;
    }
    const queryId = ++queryIdRef.current;
    // Same event-vs-response race as the main search effect: reset done-set
    // at dispatch, filter pending against it. Unlike a keystroke, the query
    // text is unchanged here, so streamed rows stay visible - the re-dispatch
    // overwrites them per extension instead of blanking the list (a content
    // watcher can fire search-invalidated while the user is idle; the
    // selection must not jump).
    doneExtsRef.current = new Set();
    invoke<SearchResponse>("search", { query: q, queryId }).then(resp => {
      if (resp.query_id !== queryIdRef.current) return;
      setResults(resp.results);
      setPendingExts(new Set(
        resp.pending.map(p => p.name).filter(n => !doneExtsRef.current.has(n)),
      ));
    }).catch(e => console.error(`[search] requery failed:`, e));
  };

  // A real extension reload may have removed extensions entirely - drop their
  // streamed rows (plain search-invalidated keeps them; see requery).
  useTauriListener("extensions-reloaded", () => {
    setStreamed(new Map());
  });

  useTauriListener("search-invalidated", () => {
    requery();
    // Also refresh content.enabled so the disabled hint appears/disappears
    // immediately when the user toggles content search in Settings.
    invoke<Config>("get_config").then(cfg => { setContentEnabled(cfg.content.enabled); setColoredIcons(cfg.files.colored_icons); configRef.current = cfg; });
  });

  const makeCtx = (): LaunchContext => ({
    setQuery,
    setResults,
    requery,
    enterClipboardMode: () => enterClipboardMode("", false),
    config: configRef.current,
  });

  const launch = (result?: SearchResult) => {
    if (!result) return;
    if (result.kind === "search-error") return;
    if (result.kind === "content-disabled") {
      invoke("open_settings_window", { section: "content" }).catch(e => console.error("[settings] open failed:", e));
      return;
    }
    if (result.kind === "content-hint") {
      enterContentMode(queryRef.current.trim());
      return;
    }
    const ctx = makeCtx();
    if (dispatchLaunch(result, ctx)) return;
    if (!result.exec) return;
    setQuery("");
    setResults([]);
    // PDFs open at the page currently shown in the preview.
    let exec = result.exec;
    const fp = result.subtitle ? `${result.subtitle}/${result.title}` : result.title;
    if (result.title.toLowerCase().endsWith(".pdf") && pdfView.path === fp) {
      exec = `xdg-open "file://${encodeURI(fp)}#page=${pdfView.page + 1}"`;
    }
    invoke("launch_app", { exec, id: result.id, kind: result.kind })
      .catch(e => console.error("[launch] launch_app failed:", e));
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // While the onboarding wizard is open it owns all input.
      if (showOnboardingRef.current) return;
      // ClipboardMode owns all key handling while the browser is open.
      if (clipboardModeRef.current) return;
      // Ignore keys when the window isn't focused - an always-on-top launcher can
      // otherwise still receive (and act on) keystrokes meant for another window.
      if (!focusedRef.current) return;
      // The action picker is modal and handles its own keys via a capture-phase
      // listener; this bubble-phase handler must stay inert while it's open.
      if (actionResult) return;
      // Quicklook is modal: result navigation must not run underneath it. Arrow
      // keys scroll the open preview (handled in QuickLook); jump keys are inert.
      if (quickResult && (
        e.key === "ArrowDown" || e.key === "ArrowUp" ||
        (e.altKey && !e.ctrlKey && !e.metaKey && e.key >= "1" && e.key <= "9")
      )) {
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        userMovedRef.current = true;
        setSelectedIndex(i => {
          const n = Math.min(i + 1, Math.max(displayResults.length - 1, 0));
          selectedIdRef.current = displayResults[n]?.id ?? null;
          return n;
        });
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        userMovedRef.current = true;
        setSelectedIndex(i => {
          const n = Math.max(i - 1, 0);
          selectedIdRef.current = displayResults[n]?.id ?? null;
          return n;
        });
      } else if (e.shiftKey && e.key === "Enter") {
        // Quicklook: pin & expand the selected file/folder preview to fill the card.
        // Placed before the plain-Enter launch so Enter alone still opens.
        e.preventDefault();
        if (quickResult) { setQuickResult(null); return; }
        const sel = displayResults[selectedIndex];
        if (sel && isPreviewable(sel)) setQuickResult(sel);
      } else if ((e.altKey && e.key === "Enter") || (e.ctrlKey && !e.altKey && !e.metaKey && (e.key === "k" || e.key === "K"))) {
        // Action picker for extension results with at least one action.
        e.preventDefault();
        const sel = quickResult ?? displayResults[selectedIndex];
        if (sel?.ext?.actions?.length) setActionResult(sel);
      } else if (e.altKey && !e.ctrlKey && !e.metaKey && e.key >= "1" && e.key <= "9") {
        e.preventDefault();
        const target = launchableResults[parseInt(e.key) - 1];
        if (!target) return;
        setSelectedIndex(displayResults.indexOf(target));
        launch(target);
      } else if (e.key === "Tab") {
        // Ghost completion wins if one is showing; otherwise Tab toggles the
        // full-text "Contents" mode, keeping the query for an in-place re-search.
        e.preventDefault();
        const ghost = ghostRef.current;
        if (ghost) { setQuery(queryRef.current + ghost); return; }
        if (contentModeRef.current) exitContentMode(); else enterContentMode();
      } else if (e.key === "Backspace" && !e.ctrlKey && !e.altKey && !e.metaKey
                 && contentModeRef.current && !queryRef.current) {
        // Empty-query Backspace drops the Contents chip, back to normal search.
        e.preventDefault();
        exitContentMode();
      } else if (e.key === "Escape") {
        e.preventDefault();
        // Esc closes the Quicklook overlay first.
        if (quickResult) { setQuickResult(null); return; }
        // In content mode, Esc backs out one level: clear the query, else drop
        // the chip. Only the bare launcher Esc hides the window.
        if (contentModeRef.current) {
          if (queryRef.current.trim()) setQuery(""); else exitContentMode();
          return;
        }
        setQuery("");
        setResults([]);
        invoke("hide_window");
      } else if (e.ctrlKey && !e.altKey && !e.metaKey
                 && (e.code === "KeyH" || e.key === "h" || e.key === "H" || e.key === "Backspace")) {
        // Toggle PDF search-term highlighting. Only meaningful in Contents mode
        // (where there are terms to highlight); otherwise let the key fall through.
        // WebKitGTK maps Ctrl+H to its "delete backward" editing command, so the
        // keydown can surface as a Backspace event (key & code both "Backspace")
        // rather than KeyH - hence the Backspace fallback, gated on Ctrl being held
        // (a bare Backspace is handled above; Ctrl+Backspace = delete-word is rare
        // in the content query and acceptable to repurpose here).
        if (!contentModeRef.current) return;
        e.preventDefault();
        setHighlight(h => !h);
      } else {
        const ctx = makeCtx();
        // While Quicklook is open, key actions target the pinned result, not
        // whatever the (hidden) selection drifted to.
        const target = quickResult ?? selected;
        if (!dispatchKeyDown(e, target, ctx)) {
          if (e.key === "Enter") {
            launch(target ?? undefined);
            if (quickResult) setQuickResult(null);
          }
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [displayResults, selectedIndex, quickResult, actionResult]);

  const selected = displayResults[selectedIndex] ?? null;

  // Dominant icon colour per result (accent bleed). The selected result's colour
  // is hoisted to a card-level var so the preview panel re-hues with the selection.
  const accents = useIconAccents(displayResults);
  const bleed = (selected ? accents.get(selected.id) : undefined) ?? undefined;

  // Quicklook is modal: blur the search input while it's open so stray typing
  // can't mutate the query/results hidden behind the overlay; refocus on close.
  // The pinned result stays put regardless of what happens to the list behind it.
  useEffect(() => {
    quicklookRef.current = quickResult != null;
    if (quickResult) inputRef.current?.blur();
    else inputRef.current?.focus();
  }, [quickResult]);

  // Action picker is modal like Quicklook: launcher input blurs while open.
  useEffect(() => {
    actionResultRef.current = actionResult != null;
    if (actionResult) inputRef.current?.blur();
    else inputRef.current?.focus();
  }, [actionResult]);

  // A reload swaps extension instances - a pinned picker would target a stale
  // provider, so drop it.
  useTauriListener("extensions-reloaded", () => setActionResult(null));

  // Toast from an extension's ShowToast activate effect (visible when the
  // window is - the backend also raises a desktop notification for the common
  // hidden-after-activate case).
  useTauriListener<string>("extension-toast", msg => {
    setToast(msg);
    window.setTimeout(() => setToast(t => (t === msg ? null : t)), 2200);
  });

  const runExtensionAction = (result: SearchResult, action: ExtAction) => {
    setActionResult(null);
    setQuery("");
    setResults([]);
    invoke("extension_activate", { id: result.id, ext: result.ext, action: action.id })
      .catch(e => console.error(`[extension] activate failed: ${e}`));
  };

  // Passive trigger-prefix affordance: when the first token of the query is a
  // registered extension prefix, annotate the input with the extension's name.
  const extensionMeta = useExtensionMeta();
  const triggerHit = useMemo(
    () => (clipboardMode || contentMode ? null : matchTrigger(query, extensionMeta)),
    [query, extensionMeta, clipboardMode, contentMode],
  );

  const calcResult = results.find(r => r.kind === "calc");
  // Whether there's an actual term to search (drives the empty state). Content
  // mode needs >= 2 chars; below that the list stays blank instead of "No results".
  const hasSearchTerm = contentMode
    ? query.trim().length >= 2
    : query.trim().length > 0;

  // Terms to highlight in the preview - only in content (full-text) mode.
  const previewTerms = useMemo(
    () => (contentMode ? deriveContentTerms(query) : []),
    [contentMode, query],
  );

  const launchableResults = useMemo(
    () => displayResults.filter(r => !NON_INDEXABLE_KINDS.has(r.kind)),
    [displayResults]
  );

  // Ghost-complete the dict word from the top prefix match in the results
  // (e.g. "define dispos" → ghosts "al" for "disposal"). The first dict result
  // is the literal typed word; the next prefix match is the completion.
  const dictGhost = useMemo(() => {
    const m = /^(?:define|dict) (\S+)$/.exec(query);
    if (!m) return null;
    const typed = m[1];
    const lc = typed.toLowerCase();
    const comp = results.find(
      r => r.kind === "dict" && r.title.toLowerCase().startsWith(lc) && r.title.toLowerCase() !== lc
    );
    return comp ? comp.title.slice(typed.length) : null;
  }, [query, results]);

  const ghostSuffix = useMemo(() => ghostFor(query) ?? dictGhost, [query, dictGhost]);
  useEffect(() => { ghostRef.current = ghostSuffix; }, [ghostSuffix]);

  const hintChip = useMemo((): string | null => {
    const q = query;
    if (q === 'define' || q === 'define ') return '<word>';
    if (q === 'dict' || q === 'dict ') return '<word>';
    if (q === 'clipboard' || q === 'clipboard ') return '<search>';
    // Bare extension trigger: mirror the define/dict placeholder treatment.
    if (triggerHit && q.trimEnd() === triggerHit.prefix) return '<query>';
    return null;
  }, [query, triggerHit]);

  const contentSized = ghostSuffix !== null || hintChip !== null || calcResult != null;

  return (
    <ColoredIconsContext.Provider value={coloredIcons}>
    <div className="launcher">
      {showOnboarding && onboardConfig && (
        <OnboardingWizard
          config={onboardConfig}
          onComplete={() => {
            setShowOnboarding(false);
            invoke<Config>("get_config").then(cfg => {
              setContentEnabled(cfg.content.enabled);
              applyTheme(cfg.appearance); // keep whatever theme the wizard saved
            });
            inputRef.current?.focus();
          }}
        />
      )}
      <div
        className="card"
        style={{ '--bleed': bleed } as CSSProperties}
        onMouseDown={e => {
          const t = e.target as HTMLElement;
          if (t !== inputRef.current && !t.closest('pre, code')) e.preventDefault();
        }}
      >
        <div className="card-clip">
        <div className="search-bar">
          {indexingProgress && (
            <div className="content-index-bar">
              <div
                className="content-index-fill"
                style={{ width: `${Math.min(100, (indexingProgress.indexed / Math.max(1, indexingProgress.total)) * 100)}%` }}
              />
            </div>
          )}
          {clipboardMode ? (
            <svg
              className="search-icon"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.9"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <rect x="8" y="2" width="8" height="4" rx="1" />
              <path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" />
            </svg>
          ) : contentMode ? (
            <svg
              className="search-icon"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.9"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
              <path d="M14 2v6h6" />
              <path d="M9 13h6" />
              <path d="M9 17h6" />
            </svg>
          ) : (
            <svg
              className="search-icon"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="11" cy="11" r="7" />
              <path d="m20 20-3.5-3.5" />
            </svg>
          )}
          <div className="search-input-area">
            {clipboardMode && <span className="search-hint-chip" style={{ marginLeft: 0, marginRight: 10 }}>Clipboard</span>}
            {contentMode && <span className="search-hint-chip" style={{ marginLeft: 0, marginRight: 10 }}>Contents</span>}
            <span ref={mirrorRef} className="input-mirror" aria-hidden="true">{query}</span>
            <input
              ref={inputRef}
              className="search-input"
              type="text"
              placeholder={clipboardMode ? "Search clipboard history…" : contentMode ? "Search file contents…" : (loading ? "Loading…" : "Search…")}
              value={query}
              onChange={e => setQuery(e.target.value)}
              autoFocus
              spellCheck={false}
              style={!clipboardMode && !contentMode && contentSized ? { width: inputWidth + 4 } : { flex: 1 }}
            />
            {!clipboardMode && !contentMode && ghostSuffix && <span className="search-ghost">{ghostSuffix}</span>}
            {!clipboardMode && !contentMode && hintChip && <span className="search-hint-chip">{hintChip}</span>}
            {!clipboardMode && !contentMode && calcResult && <div className="calc-inline">= {calcResult.title}</div>}
            {!clipboardMode && !contentMode && contentSized && <div className="search-spacer" />}
            {!clipboardMode && !contentMode && triggerHit && (
              <span className="search-hint-chip search-trigger-chip">{triggerHit.info.name}</span>
            )}
          </div>
        </div>

        {clipboardMode ? (
          <ClipboardMode
            query={query}
            capabilities={clipCaps}
            onExit={exitClipboardMode}
            onClearQuery={() => setQuery("")}
            onDeleteTag={() => { setClipboardMode(false); setQuery(""); setResults([]); inputRef.current?.focus(); }}
            onPasted={() => { setClipboardMode(false); setQuery(""); setResults([]); }}
          />
        ) : (
        <div className="body">
          <ResultsList
            results={displayResults}
            selectedIndex={selectedIndex}
            active={hasSearchTerm}
            searching={searching}
            onSelect={i => {
              userMovedRef.current = true;
              selectedIdRef.current = displayResults[i]?.id ?? null;
              setSelectedIndex(i);
            }}
            onLaunch={launch}
            launchableResults={launchableResults}
            accents={accents}
            emptyLabel={contentMode ? "No file contents match" : undefined}
            emptyReady={resolvedEmpty}
            pending={contentMode ? [] : [...pendingExts]}
          />
          <div className="preview-col">
            <PreviewPanel
              result={selected}
              onLaunch={() => launch(selected ?? undefined)}
              onReveal={() => { setQuery(""); setResults([]); }}
              terms={previewTerms}
              highlight={highlight}
            />
          </div>
        </div>
        )}

        {quickResult && !clipboardMode && (
          <QuickLook
            result={quickResult}
            terms={previewTerms}
            highlight={highlight}
            onLaunch={() => { launch(quickResult); setQuickResult(null); }}
            onClose={() => setQuickResult(null)}
          />
        )}

        {actionResult && !clipboardMode && (
          <ActionPicker
            result={actionResult}
            onRun={action => runExtensionAction(actionResult, action)}
            onClose={() => setActionResult(null)}
          />
        )}

        {toast && <div className="extension-toast">{toast}</div>}

        <div className="footer">
          <FooterHints selected={selected} canComplete={ghostSuffix !== null} quicklookOpen={quickResult != null} clipboardMode={clipboardMode} contentMode={contentMode} smartPaste={clipCaps.smart_paste} clipboardIdle={clipboardMode && query.trim() === ""} pdfHighlight={highlight} actionPickerOpen={actionResult != null} />
          <div className="footer-right">
            <button
              className="footer-settings-btn"
              onClick={() => {
                // Leave clipboard / content mode and clear the query so the launcher
                // is clean when it's next shown over (or instead of) the settings window.
                setClipboardMode(false);
                setContentMode(false);
                setQuery("");
                setResults([]);
                invoke("open_settings_window").catch(e => console.error("[settings] open failed:", e));
              }}
              title="Settings"
              tabIndex={-1}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="3"/>
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
              </svg>
            </button>
            <div className="brand">Portunus{version && <span className="brand-version">v{version}</span>}</div>
          </div>
        </div>
        </div>
      </div>
    </div>
    </ColoredIconsContext.Provider>
  );
}
