import { useState, useEffect, useLayoutEffect, useRef, useMemo, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";
import { Config, SearchResult, SearchResponse, StreamPayload, ClipboardCapabilities, ExtAction, CommandDescriptor } from "./types";
import { commandById } from "./commands/store";
import ClipboardMode from "./components/clipboard/ClipboardMode";
import { applyTheme, injectMatugenTheme } from "./theme";
import ResultsList from "./components/ResultsList";
import PreviewPanel from "./components/PreviewPanel";
import QuickLook from "./components/QuickLook";
import ActionPicker from "./components/ActionPicker";
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

const NON_INDEXABLE_KINDS = new Set(['calc', 'dict', 'dict-hint', 'content-hint', 'content-disabled', 'search-error', 'ext-error', 'command']);

// Active launcher mode: a Scope command the user entered. Null = root search.
interface ActiveMode {
  command: CommandDescriptor;
  /** Entered via `portunus --clipboard`, so Esc hides the window. */
  fromFlag?: boolean;
}

export default function App() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  // Async-tier state: batches streamed per extension for the current query,
  // and the set of extensions still working. Both keyed off queryIdRef so
  // stale events from a cancelled query can never render.
  const [streamed, setStreamed] = useState<Map<string, SearchResult[]>>(new Map());
  const [pendingExts, setPendingExts] = useState<Set<string>>(new Set());
  // Extensions whose async `query` tier failed this dispatch, keyed by name →
  // error message. Surfaced as error rows so a broken extension reads as
  // "failed - check logs" instead of silently contributing nothing.
  const [extErrors, setExtErrors] = useState<Map<string, string>>(new Map());
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

  // Single active-mode scalar: the Scope command the user is inside, or null
  // for root search. UI-takeover scopes (clipboard) swap in their own
  // component and own input handling; every other scope reuses the normal
  // results list with the search routed to the command's owner.
  const [mode, setMode] = useState<ActiveMode | null>(null);
  const modeRef = useRef<ActiveMode | null>(null);
  useEffect(() => { modeRef.current = mode; }, [mode]);
  const isTakeover = (m: ActiveMode | null) => m?.command.route.type === "ui_takeover";
  const inClipboard = isTakeover(mode);
  const inContents = mode?.command.id === "cmd:contents";
  // A "browse" scope (min_query_len 0) renders its list the moment it's entered,
  // with an empty query - used by dashboard-style commands (e.g. an extension's
  // "My Pull Requests"). Every other scope stays blank until the user types.
  const browsing = !!mode && !inClipboard && !inContents && (mode.command.min_query_len ?? 1) === 0;
  const [clipCaps, setClipCaps] = useState<ClipboardCapabilities>({ smart_paste: false });
  const capsFetched = useRef(false);

  // Enters a Scope command's mode. `seed` pre-fills the scoped query (Tab
  // keeps the typed text so a search flips between name and content matching;
  // entry launches pass "").
  const enterMode = (command: CommandDescriptor, opts: { seed?: string; fromFlag?: boolean } = {}) => {
    if (command.route.type === "ui_takeover" && !capsFetched.current) {
      capsFetched.current = true;
      invoke<ClipboardCapabilities>("clipboard_capabilities").then(setClipCaps).catch(() => {});
    }
    setMode({ command, fromFlag: opts.fromFlag });
    if (opts.seed !== undefined) setQuery(opts.seed);
    setResults([]);
    setResolvedEmpty(false);
    inputRef.current?.focus();
  };

  const enterClipboardMode = (seed = "", fromFlag = false) => {
    const cmd = commandById("cmd:clipboard");
    if (cmd) enterMode(cmd, { seed, fromFlag });
  };

  const enterContentMode = (seed?: string) => {
    const cmd = commandById("cmd:contents");
    if (cmd) enterMode(cmd, { seed });
  };

  // Leaves whatever mode is active. Takeover scopes clear the query (their
  // filter is mode-local); others keep it so the search re-runs at root.
  const exitMode = () => {
    const m = modeRef.current;
    setMode(null);
    setResults([]);
    setResolvedEmpty(false);
    if (isTakeover(m)) {
      setQuery("");
      if (m?.fromFlag) { invoke("hide_window"); return; }
    }
    inputRef.current?.focus();
  };

  // Invokes a command entry (Enter on a `kind: "command"` row).
  const runCommand = (command: CommandDescriptor) => {
    // Frecency: frequently-used commands float up in root search.
    invoke("command_used", { id: command.id }).catch(() => {});
    if (command.mode_kind === "scope") {
      enterMode(command, { seed: "" });
      return;
    }
    // Action command: one-shot - run the extension's activate with the
    // command name and a synthetic default result (there's no search result
    // behind an entry row). The backend hides the window first.
    if (command.route.type === "extension") {
      setQuery("");
      setResults([]);
      invoke("extension_activate", {
        id: command.id,
        ext: { id: command.route.command, title: command.title, relevance: 0 },
        action: null,
        command: command.route.command,
      }).catch(e => console.error(`[command] action failed: ${e}`));
    }
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
    setMode(null);
    setActionResult(null);
    setQuery("");
    setResults([]);
    setStreamed(new Map());
    setPendingExts(new Set());
    setExtErrors(new Map());
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
    if (mode) return;
    const m = /^(clip|clipb|clipbo|clipboa|clipboar|clipboard)\s+(.*)$/i.exec(query);
    if (m) enterClipboardMode(m[2], false);
  }, [query, mode]);

  // Refs mirroring the async-tier state for use inside the (deliberately
  // dependency-light) search effect below.
  const streamedRef = useRef(false);
  const pendingRef = useRef(false);
  useEffect(() => { streamedRef.current = streamed.size > 0; }, [streamed]);
  useEffect(() => { pendingRef.current = pendingExts.size > 0; }, [pendingExts]);

  useEffect(() => {
    if (inClipboard) { setSearching(false); return; }
    const trimmed = query.trim();
    // A scope's min_query_len (e.g. contents needs >= 2 chars, matching the
    // backend guard) gates the dispatch; below it, clear and show the prompt.
    // Browse scopes dispatch on entry with an empty query (they list first,
    // filter as you type); every other context needs at least one character.
    const minLen = browsing ? 0 : Math.max(1, mode?.command.min_query_len ?? 1);
    if ((!trimmed && !browsing) || trimmed.length < minLen) {
      setResults([]); setSearching(false); setResolvedEmpty(false);
      // Nothing to search = nothing the async tier should keep working on.
      if (streamedRef.current || pendingRef.current) {
        setStreamed(new Map());
        setPendingExts(new Set());
        setExtErrors(new Map());
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
    const debounceMs = inContents ? 90 : 10;
    // A dead prefix mid-word (or the porter stem gap prefix matching can't cover)
    // resolves to zero for a beat before the next keystroke matches. Hold the
    // empty verdict behind this settle so the list stays neutral-blank instead of
    // flashing "no results" while the user is still typing.
    const EMPTY_SETTLE_MS = 220;
    const t = setTimeout(() => {
      const queryId = ++queryIdRef.current;
      // New dispatch: everything streamed belongs to an older query now.
      // Clear here, not in .then - the backend dispatches async workers
      // before the search response resolves, so a fast extension's first
      // batch (or done signal) can legitimately arrive ahead of it.
      setStreamed(new Map());
      setExtErrors(new Map());
      doneExtsRef.current = new Set();
      const scope = mode?.command.id ?? null;
      invoke<SearchResponse>("search", { query, queryId, scope }).then(resp => {
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
  }, [query, mode]);

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
      // Track (or clear) the failure so a re-run that now succeeds drops the
      // stale error row.
      setExtErrors(prev => {
        const has = prev.has(p.ext);
        if (p.error) {
          if (prev.get(p.ext) === p.error) return prev;
          const next = new Map(prev);
          next.set(p.ext, p.error);
          return next;
        }
        if (!has) return prev;
        const next = new Map(prev);
        next.delete(p.ext);
        return next;
      });
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
    if (mode || searching || searchError) return;
    if (!query.trim()) return;
    if (pendingExts.size > 0 || results.length > 0 || streamed.size > 0) return;
    const t = setTimeout(() => setResolvedEmpty(true), 220);
    return () => clearTimeout(t);
  }, [pendingExts, results, streamed, searching, searchError, query, mode]);

  useEffect(() => {
    setSelectedIndex(0);
    selectedIdRef.current = null;
    userMovedRef.current = false;
  }, [query, mode]);

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

  // Synthetic rows for extensions whose async query failed. Sorted by name so
  // their order is stable as failures land, and appended after real results.
  const extErrorRows = useMemo<SearchResult[]>(
    () =>
      [...extErrors.entries()]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([name, message]) => ({
          id: `ext-error:${name}`,
          title: `${name} failed`,
          subtitle: message || "Extension error - check its logs in Settings",
          kind: "ext-error",
          score: 0,
        })),
    [extErrors],
  );

  const displayResults = useMemo<SearchResult[]>(() => {
    if (!query.trim()) {
      // Browse scopes show their streamed list immediately; failed-extension
      // rows still pin to the bottom. Everything else stays blank when empty.
      if (browsing) return extErrorRows.length > 0 ? [...merged, ...extErrorRows] : merged;
      return [];
    }
    if (searchError) {
      return [{
        id: "search:error",
        title: "Search failed",
        subtitle: "Something went wrong - check the logs and try again",
        kind: "search-error",
        score: 0,
      }];
    }
    if (inContents) {
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
      // A failing extension with no other results: show the error rows rather
      // than the "search file contents" hint, which would misread the failure
      // as "nothing matched".
      if (extErrorRows.length > 0) return extErrorRows;
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
    // Real results first, failed-extension rows pinned to the bottom.
    return extErrorRows.length > 0 ? [...merged, ...extErrorRows] : merged;
  }, [query, results, merged, extErrorRows, contentEnabled, resolvedEmpty, inContents, searchError, browsing]);

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
    const scope = modeRef.current?.command.id ?? null;
    if (modeRef.current && isTakeover(modeRef.current)) return;
    const queryId = ++queryIdRef.current;
    // Same event-vs-response race as the main search effect: reset done-set
    // at dispatch, filter pending against it. Unlike a keystroke, the query
    // text is unchanged here, so streamed rows stay visible - the re-dispatch
    // overwrites them per extension instead of blanking the list (a content
    // watcher can fire search-invalidated while the user is idle; the
    // selection must not jump).
    doneExtsRef.current = new Set();
    invoke<SearchResponse>("search", { query: q, queryId, scope }).then(resp => {
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
    setExtErrors(new Map());
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
    runCommand,
    config: configRef.current,
  });

  const launch = (result?: SearchResult) => {
    if (!result) return;
    if (result.kind === "search-error") return;
    if (result.kind === "ext-error") {
      invoke("open_settings_window", { section: "extensions" }).catch(e => console.error("[settings] open failed:", e));
      return;
    }
    if (result.kind === "content-disabled") {
      invoke("open_settings_window", { section: "content" }).catch(e => console.error("[settings] open failed:", e));
      return;
    }
    if (result.kind === "content-hint") {
      enterContentMode(queryRef.current.trim());
      return;
    }
    if (result.kind === "command" && result.command) {
      runCommand(result.command);
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
      if (isTakeover(modeRef.current)) return;
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
        // Tab toggles the full-text "Contents" mode, keeping the query for an
        // in-place re-search.
        e.preventDefault();
        if (modeRef.current?.command.id === "cmd:contents") exitMode();
        else if (!modeRef.current) enterContentMode();
      } else if (e.key === "Backspace" && !e.ctrlKey && !e.altKey && !e.metaKey
                 && modeRef.current && !queryRef.current) {
        // Empty-query Backspace drops the mode chip, back to normal search.
        e.preventDefault();
        exitMode();
      } else if (e.key === "Escape") {
        e.preventDefault();
        // Esc closes the Quicklook overlay first.
        if (quickResult) { setQuickResult(null); return; }
        // In a mode, Esc backs out one level: clear the query, else drop the
        // chip. Only the bare launcher Esc hides the window.
        if (modeRef.current) {
          if (queryRef.current.trim()) setQuery(""); else exitMode();
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
        if (modeRef.current?.command.id !== "cmd:contents") return;
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
    invoke("extension_activate", { id: result.id, ext: result.ext, action: action.id, command: result.ext_command ?? null })
      .catch(e => console.error(`[extension] activate failed: ${e}`));
  };

  const calcResult = results.find(r => r.kind === "calc");
  // Whether there's an actual term to search (drives the empty state). Content
  // mode needs >= 2 chars; below that the list stays blank instead of "No results".
  const hasSearchTerm = inContents
    ? query.trim().length >= 2
    : browsing || query.trim().length > 0;

  // Terms to highlight in the preview - only in content (full-text) mode.
  const previewTerms = useMemo(
    () => (inContents ? deriveContentTerms(query) : []),
    [inContents, query],
  );

  const launchableResults = useMemo(
    () => displayResults.filter(r => !NON_INDEXABLE_KINDS.has(r.kind)),
    [displayResults]
  );

  const contentSized = calcResult != null;

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
          {inClipboard ? (
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
          ) : inContents ? (
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
            {mode && <span className="search-hint-chip" style={{ marginLeft: 0, marginRight: 10 }}>{mode.command.chip}</span>}
            <span ref={mirrorRef} className="input-mirror" aria-hidden="true">{query}</span>
            <input
              ref={inputRef}
              className="search-input"
              type="text"
              placeholder={mode ? (mode.command.placeholder ?? `Search ${mode.command.title.toLowerCase()}…`) : (loading ? "Loading…" : "Search…")}
              value={query}
              onChange={e => setQuery(e.target.value)}
              autoFocus
              spellCheck={false}
              style={!mode && contentSized ? { width: inputWidth + 4 } : { flex: 1 }}
            />
            {!mode && calcResult && <div className="calc-inline">= {calcResult.title}</div>}
            {!mode && contentSized && <div className="search-spacer" />}
          </div>
        </div>

        {inClipboard ? (
          <ClipboardMode
            query={query}
            capabilities={clipCaps}
            onExit={exitMode}
            onClearQuery={() => setQuery("")}
            onDeleteTag={() => { setMode(null); setQuery(""); setResults([]); inputRef.current?.focus(); }}
            onPasted={() => { setMode(null); setQuery(""); setResults([]); }}
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
            emptyLabel={inContents ? "No file contents match" : undefined}
            emptyReady={resolvedEmpty}
            pending={inContents ? [] : [...pendingExts]}
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

        {quickResult && !inClipboard && (
          <QuickLook
            result={quickResult}
            terms={previewTerms}
            highlight={highlight}
            onLaunch={() => { launch(quickResult); setQuickResult(null); }}
            onClose={() => setQuickResult(null)}
          />
        )}

        {actionResult && !inClipboard && (
          <ActionPicker
            result={actionResult}
            onRun={action => runExtensionAction(actionResult, action)}
            onClose={() => setActionResult(null)}
          />
        )}

        {toast && <div className="extension-toast">{toast}</div>}

        <div className="footer">
          <FooterHints selected={selected} quicklookOpen={quickResult != null} clipboardMode={inClipboard} contentMode={inContents} smartPaste={clipCaps.smart_paste} clipboardIdle={inClipboard && query.trim() === ""} pdfHighlight={highlight} actionPickerOpen={actionResult != null} />
          <div className="footer-right">
            <button
              className="footer-settings-btn"
              onClick={() => {
                // Leave any active mode and clear the query so the launcher is
                // clean when it's next shown over (or instead of) the settings window.
                setMode(null);
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
