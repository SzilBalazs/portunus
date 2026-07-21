import { useState, useEffect, useLayoutEffect, useRef, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Config, SearchResult, SearchResponse, StreamPayload, ClipboardCapabilities, CommandDescriptor, ActivateResponse, ExtensionResult, FormDto, ToastLevel } from "./types";
import { commandById } from "./commands/store";
import ClipboardMode from "./components/clipboard/ClipboardMode";
import { applyTheme, injectMatugenTheme } from "./theme";
import ResultsList from "./components/ResultsList";
import PreviewPanel from "./components/PreviewPanel";
import QuickLook from "./components/QuickLook";
import ActionPanel from "./components/ActionPanel";
import ExtensionFormModal from "./components/ExtensionFormModal";
import { deriveContentTerms } from "./highlight";
import FooterHints from "./components/FooterHints";
import { pdfView } from "./components/FilePreview";
import { isPreviewable } from "./utils";
import { ColoredIconsContext } from "./coloredIcons";
import { dispatchLaunch, dispatchShortcut, collectResultActions, isCopyKey, type LaunchContext } from "./providers/registry";
import { getKeybinds, matchesBuiltin, useKeybinds } from "./keybinds/store";
import { eventToChord } from "./keybinds/chord";
import type { ActionDescriptor } from "./actions/types";
import SelectionLayer from "./selection/SelectionLayer";
import { selection } from "./selection/controller";
import { useTauriListener } from "./hooks/useTauriListener";
import OnboardingWizard from "./components/onboarding/OnboardingWizard";
import "./providers";
import "./App.css";
import "./themes.css";

const NON_INDEXABLE_KINDS = new Set(['calc', 'dict', 'dict-hint', 'content-hint', 'content-disabled', 'search-error', 'ext-error', 'command']);

// Shared stable empty terms array - handed to the preview outside content mode
// so its identity never changes across keystrokes (a fresh [] would churn the
// preview subtree).
const EMPTY_TERMS: string[] = [];

// Optimistic-hide budget for extension activations: dismiss feels instant,
// yet a fast response can still keep the window open (form/keep-open).
const ACTIVATE_HIDE_MS = 150;

/** One activation routed through the shared response flow. */
interface ExtActivateRequest {
  id: string;
  ext: ExtensionResult;
  action: string | null;
  command: string | null;
  /** Present on form submits; its presence marks the call as form-originated. */
  formValues?: Record<string, unknown>;
  /** The action/command declared `opens_form` - skip the optimistic hide. */
  opensForm?: boolean;
}

interface ActiveToast {
  id: number;
  message: string;
  level: ToastLevel;
}

/** Form pinned by a ShowForm activate effect, with what's needed to submit. */
interface ActiveExtForm {
  id: string;
  ext: ExtensionResult;
  command: string | null;
  form: FormDto;
}

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
  // The action panel (Alt+Enter / Ctrl+K; null = closed). Pins the result it
  // was opened for - like Quicklook - so background reorders can't retarget
  // the actions. `result` is null when opened with nothing selected, which
  // shows only the global section.
  const [actionPanel, setActionPanel] = useState<{ result: SearchResult | null } | null>(null);
  // Toast queue fed by extension activate responses (newest at the bottom,
  // capped at 3). Errors linger longer and are click-dismissable like the rest.
  const [toasts, setToasts] = useState<ActiveToast[]>([]);
  const toastIdRef = useRef(0);
  // Form pinned by a ShowForm activate effect (null = closed). Modal like the
  // action picker; pins the result so background reorders can't retarget it.
  const [extForm, setExtForm] = useState<ActiveExtForm | null>(null);
  // True while a form submit's activate call is in flight (locks the form).
  const [formBusy, setFormBusy] = useState(false);
  // True while an opens_form activation runs with the window kept visible
  // (no optimistic hide) - drives the "Working…" pill so the launcher
  // doesn't look frozen during the extension's network I/O.
  const [activatePending, setActivatePending] = useState(false);

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

  // Selection actions shared by Ctrl+F and the selection popover: throw the
  // selected text into the search bar / the dict scope as a fresh lookup.
  const searchFromSelection = (raw: string) => {
    const text = raw.replace(/\s+/g, " ").trim();
    selection.clear();
    if (!text) return;
    setQuickResult(null);
    setQuery(text);
    inputRef.current?.focus();
  };

  const defineFromSelection = (word: string) => {
    selection.clear();
    setQuickResult(null);
    const cmd = commandById("cmd:dict");
    if (cmd) enterMode(cmd, { seed: word.toLowerCase() });
  };

  // Invokes a command entry (Enter on a `kind: "command"` row).
  const runCommand = (command: CommandDescriptor) => {
    // Frecency: frequently-used commands float up in root search.
    invoke("command_used", { id: command.id }).catch(() => {});
    if (command.mode_kind === "scope") {
      enterMode(command, { seed: "" });
      return;
    }
    // Built-in action: invoke the named Tauri command directly. The launcher
    // is cleaned up so it's fresh when next shown (the command may hide it,
    // e.g. open_settings_window hides "main" backend-side).
    if (command.route.type === "invoke") {
      const tauriCmd = command.route.command;
      const args = command.route.args ?? {};
      setMode(null);
      setQuery("");
      setResults([]);
      invoke(tauriCmd, args).catch(e => console.error(`[command] invoke ${tauriCmd} failed:`, e));
      return;
    }
    // Action command: one-shot - run the extension's activate with the
    // command name and a synthetic default result (there's no search result
    // behind an entry row).
    if (command.route.type === "extension") {
      activateExtension({
        id: command.id,
        ext: { id: command.route.command, title: command.title, relevance: 0 },
        action: null,
        command: command.route.command,
        opensForm: command.opens_form === true,
      });
    }
  };

  useEffect(() => { queryRef.current = query; }, [query]);

  // Alt-held tracker: Alt+1..9 badges render only while Alt is down. Lives in
  // its own effect (not the main keydown handler, which early-returns in
  // clipboard/onboarding modes - badges must work there too) and writes the
  // attribute imperatively so holding Alt never re-renders the tree.
  // Alt+digit hides the window while Alt is still down, so the keyup lands in
  // another app - blur/visibility/focus handlers clear the stuck state.
  useEffect(() => {
    const root = document.documentElement;
    const set = (held: boolean) => {
      if (held) {
        if (root.dataset.altHeld !== "true") root.dataset.altHeld = "true";
      } else if (root.dataset.altHeld) {
        delete root.dataset.altHeld;
      }
    };
    const onKeyDown = (e: KeyboardEvent) => { if (e.altKey) set(true); };
    const onKeyUp = (e: KeyboardEvent) => { if (e.key === "Alt" || !e.altKey) set(false); };
    const onClear = () => set(false);
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", onClear);
    document.addEventListener("visibilitychange", onClear);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onClear);
      document.removeEventListener("visibilitychange", onClear);
    };
  }, []);

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
    // The backend emits progress every 10 files via a parallel walk - hundreds
    // per second on a large index. Coalesce to one state write per frame so the
    // storm can't peg the main thread re-rendering the tree (e.g. an open
    // markdown preview). The completion event still lands immediately so the bar
    // always resolves.
    let pending: { indexed: number; total: number } | null = null;
    let raf = 0;
    const flush = () => {
      raf = 0;
      if (!pending) return;
      const p = pending;
      pending = null;
      setIndexingProgress(p);
      clearTimeout(doneTimer);
      if (p.indexed >= p.total && p.total > 0) {
        doneTimer = setTimeout(() => setIndexingProgress(null), 150);
      }
    };
    listen<{ indexed: number; total: number }>("content-index-progress", event => {
      pending = event.payload;
      if (!raf) raf = requestAnimationFrame(flush);
    }).then(fn => {
      if (active) unlisten = fn; else fn();
    });
    return () => { active = false; if (raf) cancelAnimationFrame(raf); clearTimeout(doneTimer); unlisten?.(); };
  }, []);

  useTauriListener("window-show", () => {
    focusedRef.current = true;
    delete document.documentElement.dataset.altHeld;
    selection.clear();
    // A plain `--show` always opens the clean launcher, never a stale clipboard
    // session. (`--clipboard` uses window-show-query, which re-enters the mode.)
    setMode(null);
    setActionPanel(null);
    setExtForm(null);
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
      if (!focused) {
        delete document.documentElement.dataset.altHeld;
        selection.clear();
      }
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
      // A volatile scope (a live queue) emits its full result set per batch and
      // its membership changes underneath us, so replace that extension's rows
      // wholesale: the fresh batch swaps in atomically (no pre-clear, so no
      // blank frame) and rows that dropped out don't linger. Non-volatile
      // batches merge by id - the "cached now, fresh later" update-in-place
      // pattern. The keystroke path clears `streamed` before dispatch (see the
      // search effect), so merging into an empty map is identical to replacing;
      // a plain requery does NOT clear, so merging keeps rows visible and
      // updates in place instead of blanking the list and jumping the selection.
      const volatileScope = modeRef.current?.command.volatile === true;
      setStreamed(prev => {
        const next = new Map(prev);
        if (volatileScope) {
          next.set(p.ext, p.results);
        } else {
          const byId = new Map((next.get(p.ext) ?? []).map(r => [r.id, r] as const));
          for (const r of p.results) byId.set(r.id, r);
          next.set(p.ext, [...byId.values()]);
        }
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

  const requery = (opts?: { clear?: boolean }) => {
    const q = queryRef.current;
    const m = modeRef.current;
    if (m && isTakeover(m)) return;
    // Browse scopes (min_query_len 0, e.g. marketplace) dispatch on an empty
    // query; every other context needs a non-empty one. Without this, an
    // install/uninstall in the marketplace browse scope wouldn't re-run the
    // scoped search, leaving the row's state (and its preview/actions) stale.
    // Derive from modeRef, not the closed-over `browsing`: the
    // search-invalidated listener captures the first-render requery, where
    // `browsing` is stale-false.
    const browseScope =
      !!m && m.command.id !== "cmd:contents" && (m.command.min_query_len ?? 1) === 0;
    if (!q.trim() && !browseScope) return;
    const scope = m?.command.id ?? null;
    const queryId = ++queryIdRef.current;
    // Same event-vs-response race as the main search effect: reset done-set
    // at dispatch, filter pending against it. Unlike a keystroke, the query
    // text is unchanged here, so streamed rows stay visible - the re-dispatch
    // overwrites them per extension instead of blanking the list (a content
    // watcher can fire search-invalidated while the user is idle; the
    // selection must not jump).
    //
    // `clear` re-renders from scratch for a non-volatile mutation (delete/
    // toggle) whose result set can shrink: merge-by-id never drops rows, so
    // stale ones would linger without a clear. A volatile scope does NOT
    // pre-clear here - it replaces its rows per batch in the stream handler,
    // which swaps atomically with no blank frame. Clearing a volatile scope
    // instead would blank the list for the bus round-trip (a visible flash),
    // since its instant tier is empty and all rows arrive via the stream.
    if (opts?.clear && !m?.command.volatile) {
      setStreamed(new Map());
      setExtErrors(new Map());
    }
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

  // Entering the marketplace scope force-refreshes the index so the browse list
  // reflects the live catalog before the user can act (a stale cache serves
  // moved shas / pulled versions that only fail at install). Cheap: the ETag
  // makes an unchanged index a 304 with no re-download; a changed index emits
  // marketplace-index-updated + search-invalidated, which re-runs the query.
  useEffect(() => {
    if (mode?.command.id === "cmd:marketplace") {
      invoke("marketplace_refresh", { force: true }).catch(() => {});
    }
  }, [mode?.command.id]);

  const pushToast = (message: string, level: ToastLevel) => {
    const id = ++toastIdRef.current;
    setToasts(ts => [...ts.slice(-2), { id, message, level }]);
    window.setTimeout(
      () => setToasts(ts => ts.filter(t => t.id !== id)),
      level === "error" ? 5000 : 2200,
    );
  };

  // Pins are a root-search concept: a real typed query, and a result kind
  // that stays addressable across sessions (no calc/dict/hint/error rows).
  const canPin = (result: SearchResult): boolean => {
    if (modeRef.current || !queryRef.current.trim()) return false;
    if (result.id.startsWith("ext:") && result.ext) return true;
    return ["app", "file", "folder", "command"].includes(result.kind);
  };

  // Pin/unpin the result for the current query (Ctrl+P or the action panel).
  // Unpin removes every pin currently boosting the row for what's typed.
  const togglePin = (result: SearchResult) => {
    const q = queryRef.current.trim();
    const call = result.pinned
      ? invoke("unpin_result", { query: q, resultId: result.id })
      : invoke("pin_result", {
          query: q,
          result: {
            id: result.id,
            kind: result.kind,
            title: result.title,
            subtitle: result.subtitle ?? null,
          },
        });
    call
      .then(() => {
        pushToast(result.pinned ? "Unpinned" : `Pinned for “${q}”`, "success");
        requery();
      })
      .catch(e => pushToast(String(e), "error"));
  };

  const reshowWindow = () => {
    const win = getCurrentWindow();
    win.show().then(() => win.setFocus()).catch(() => {});
  };

  // Every extension activation (Enter, action picker, action command, form
  // submit) funnels through here. The window hides optimistically after
  // ACTIVATE_HIDE_MS so dismissal feels instant even when the extension does
  // network I/O; the response then settles visibility (a form or keep-open
  // re-shows), feeds the toast queue, and requeries when asked to.
  const activateExtension = (req: ExtActivateRequest) => {
    const fromForm = req.formValues !== undefined;
    if (fromForm) setFormBusy(true);
    let hidden = false;
    // No optimistic hide behind an open form - it would yank the modal away.
    // Same when the action/command declared opens_form: hiding would flash
    // the window hidden-then-shown around the arriving form, so the window
    // stays up and a "Working…" pill shows while the extension runs.
    const skipHide = fromForm || req.opensForm === true;
    if (skipHide && !fromForm) setActivatePending(true);
    const timer = skipHide
      ? undefined
      : window.setTimeout(() => {
          hidden = true;
          invoke("hide_window");
        }, ACTIVATE_HIDE_MS);
    invoke<ActivateResponse>("extension_activate", {
      id: req.id,
      ext: req.ext,
      action: req.action,
      command: req.command,
      formValues: req.formValues ?? null,
    }).then(resp => {
      if (timer !== undefined) clearTimeout(timer);
      setFormBusy(false);
      setActivatePending(false);
      for (const t of resp.toasts) pushToast(t.message, t.level);
      // Replace the launcher query (e.g. drill-down menuing). The value may
      // equal the current text (clearing an already-empty box), so setQuery
      // alone wouldn't re-fire the search effect - sync the ref and force a
      // requery. Skipped when the window is hiding (resp.hide clears the box
      // below and wins). requery() already tolerates empty in browse scopes.
      if (resp.setQuery !== null && !resp.hide) {
        queryRef.current = resp.setQuery;
        setQuery(resp.setQuery);
        requery();
      }
      if (resp.refreshResults) requery({ clear: true });
      // Drill-in into a new list: force the cursor back to the top. The
      // [query] reset effect only fires when the query text actually changes,
      // so a SetQuery("") on an already-empty box wouldn't reset it - clear the
      // pin and select row 0 explicitly.
      if (resp.selectFirst) {
        selectedIdRef.current = null;
        userMovedRef.current = false;
        setSelectedIndex(0);
      }
      if (resp.form) {
        setExtForm({ id: req.id, ext: req.ext, command: req.command, form: resp.form });
        if (hidden) reshowWindow();
        return;
      }
      setExtForm(null);
      if (resp.hide) {
        setQuery("");
        setResults([]);
        if (!hidden) invoke("hide_window");
      } else if (hidden) {
        // A slow keep-open activation lost the race against the timer.
        reshowWindow();
      }
    }).catch(e => {
      if (timer !== undefined) clearTimeout(timer);
      setFormBusy(false);
      setActivatePending(false);
      console.error(`[extension] activate failed: ${e}`);
      // Visible when the window is (form submits, keep-open flows); a hidden
      // launcher's failure still lands in the extension's Settings log.
      pushToast(String(e), "error");
    });
  };

  const makeCtx = (): LaunchContext => ({
    setQuery,
    setResults,
    requery,
    pushToast,
    runCommand,
    activateExtension,
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

  // Quicklook toggle shared by the Shift+Enter branch and the panel's
  // "Quick Look" row, so the chord and the menu can't drift apart.
  const toggleQuickLook = (sel: SearchResult | null) => {
    if (quickResult) { setQuickResult(null); return; }
    if (sel && isPreviewable(sel)) setQuickResult(sel);
  };

  useEffect(() => {
    // Remappable built-in chords, dispatched by effective chord (defaults
    // overlaid with [keybinds.builtin]). A handler returns false to fall
    // through - e.g. the highlight chord outside Contents mode must reach
    // WebKitGTK's native editing commands untouched.
    const builtinHandlers: Record<string, () => boolean> = {
      "builtin:quick-look": () => {
        // Quicklook: pin & expand the selected file/folder preview to fill
        // the card. Enter alone still opens (fixed launch fallback).
        toggleQuickLook(displayResults[selectedIndex] ?? null);
        return true;
      },
      "builtin:action-panel": () => {
        // Available for every result kind (and with nothing selected, where
        // it shows just the global section).
        setActionPanel({ result: quickResult ?? displayResults[selectedIndex] ?? null });
        return true;
      },
      "builtin:contents": () => {
        // Toggles the full-text "Contents" mode, keeping the query for an
        // in-place re-search; inert inside other modes.
        if (modeRef.current?.command.id === "cmd:contents") { exitMode(); return true; }
        if (!modeRef.current) { enterContentMode(); return true; }
        return false;
      },
      "builtin:pin": () => {
        // Pin/unpin the selected result for the typed query. Swallow the
        // chord regardless so WebKitGTK's print shortcut never fires.
        const target = quickResult ?? selected;
        if (target && canPin(target)) togglePin(target);
        return true;
      },
      "builtin:highlight": () => {
        // Toggle PDF search-term highlighting. Only meaningful in Contents
        // mode (where there are terms to highlight); otherwise fall through.
        if (modeRef.current?.command.id !== "cmd:contents") return false;
        setHighlight(h => !h);
        return true;
      },
    };
    const onKey = (e: KeyboardEvent) => {
      // While the onboarding wizard is open it owns all input.
      if (showOnboardingRef.current) return;
      // ClipboardMode owns all key handling while the browser is open.
      if (isTakeover(modeRef.current)) return;
      // Ignore keys when the window isn't focused - an always-on-top launcher can
      // otherwise still receive (and act on) keystrokes meant for another window.
      if (!focusedRef.current) return;
      // The action panel and extension form are modal and handle their own
      // keys via capture-phase listeners; this bubble-phase handler must stay
      // inert while either is open.
      if (actionPanel || extForm) return;
      // Quicklook is modal: result navigation must not run underneath it. Arrow
      // keys scroll the open preview (handled in QuickLook); jump keys are inert.
      if (quickResult && (
        e.key === "ArrowDown" || e.key === "ArrowUp" ||
        (e.altKey && !e.ctrlKey && !e.metaKey && e.key >= "1" && e.key <= "9")
      )) {
        return;
      }
      // Preview text selection wins the copy chord; without one it falls
      // through to the provider handlers (copy path / calc result / …).
      if (isCopyKey(e) && selection.hasSelection()) {
        e.preventDefault();
        selection.copy();
        return;
      }
      // Search-selection throws the selected text back into the search bar as
      // a fresh query - select a word in a preview, instantly search it.
      if (matchesBuiltin(e, "builtin:search-selection") && selection.hasSelection()) {
        e.preventDefault();
        searchFromSelection(selection.getText());
        return;
      }
      // Keyboard select mode owns the movement keys (its capture listener
      // consumes them; this guard is belt-and-braces for result navigation).
      if (selection.isKeyboardMode()
          && (e.key.startsWith("Arrow") || e.key === "Home" || e.key === "End")) {
        return;
      }
      // Select-mode chord enters keyboard text selection in the visible preview.
      if (matchesBuiltin(e, "builtin:select-mode")) {
        const root = document.querySelector<HTMLElement>(
          quickResult ? ".quicklook-overlay [data-selectable]" : ".card [data-selectable]",
        );
        if (selection.enterKeyboardMode(root)) {
          e.preventDefault();
          return;
        }
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
      } else if (e.altKey && !e.ctrlKey && !e.metaKey && e.key >= "1" && e.key <= "9") {
        e.preventDefault();
        const target = launchableResults[parseInt(e.key) - 1];
        if (!target) return;
        setSelectedIndex(displayResults.indexOf(target));
        launch(target);
      } else if (e.key === "Backspace" && !e.ctrlKey && !e.altKey && !e.metaKey
                 && modeRef.current && !queryRef.current) {
        // Empty-query Backspace drops the mode chip, back to normal search.
        e.preventDefault();
        exitMode();
      } else if (e.key === "Escape") {
        e.preventDefault();
        // Esc dismisses a preview text selection before anything else.
        if (selection.hasSelection() || selection.isKeyboardMode()) {
          selection.clear();
          return;
        }
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
      } else {
        // ── layered chord dispatch: result actions → global command bindings
        // → (remappable) built-ins → plain-Enter launch. Effective shortcuts
        // (user overrides from [keybinds]) are applied inside the registry.
        //
        // Plain Tab never moves focus out of the search input, whether it is
        // still bound to Contents mode or remapped away.
        if (e.key === "Tab" && !e.ctrlKey && !e.altKey) e.preventDefault();
        const ctx = makeCtx();
        // While Quicklook is open, key actions target the pinned result, not
        // whatever the (hidden) selection drifted to.
        const target = quickResult ?? selected;
        // Key repeat is suppressed for actions/commands (holding a queue
        // chord must not fire five times); built-ins are harmless toggles.
        if (!e.repeat && dispatchShortcut(e, target, ctx)) return;
        const chord = eventToChord(e);
        if (chord && !e.repeat) {
          const cmdId = getKeybinds().commandChords.get(chord);
          const cmd = cmdId ? commandById(cmdId) : undefined;
          if (cmd) {
            e.preventDefault();
            runCommand(cmd);
            return;
          }
        }
        if (chord) {
          const bId = getKeybinds().builtinChords.get(chord);
          if (bId && builtinHandlers[bId]?.()) {
            e.preventDefault();
            return;
          }
        }
        if (e.key === "Enter") {
          launch(target ?? undefined);
          if (quickResult) setQuickResult(null);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [displayResults, selectedIndex, quickResult, actionPanel, extForm]);

  const selected = displayResults[selectedIndex] ?? null;

  // Selecting a different result or switching modes swaps the previewed content
  // out from under a text selection - drop it. Query edits are NOT a dependency:
  // the input keeps focus during a selection, so a stray keystroke must not wipe
  // it before Ctrl+C; when a query change actually re-renders the preview, the
  // controller's MutationObserver clears the now-disconnected selection anyway.
  useEffect(() => {
    selection.clear();
  }, [mode, selected?.id]);

  // Toggling quicklook swaps the preview between two DOM subtrees that render the
  // same text (side panel ⇄ overlay). Carry the selection across instead of
  // dropping it; fall back to clear if the now-visible root has no matching text
  // yet (e.g. an async preview). Layout effect: the target subtree is committed.
  useLayoutEffect(() => {
    if (!selection.hasSelection() && !selection.isKeyboardMode()) return;
    const cap = selection.captureLinear();
    if (!cap) { selection.clear(); return; }
    const sel = quickResult ? ".quicklook-overlay [data-selectable]" : ".card [data-selectable]";
    // Async previews (md/pdf/png text layers) populate a few frames after mount;
    // poll until the target has matching text, then remap. Give up after ~1s.
    let raf = 0;
    let tries = 0;
    const attempt = () => {
      const root = document.querySelector<HTMLElement>(sel);
      if (root && selection.applyLinear(root, cap)) return;
      if (++tries > 60) { selection.clear(); return; }
      raf = requestAnimationFrame(attempt);
    };
    attempt();
    return () => cancelAnimationFrame(raf);
  }, [quickResult?.id]);

  // Quicklook is modal: blur the search input while it's open so stray typing
  // can't mutate the query/results hidden behind the overlay; refocus on close.
  // The pinned result stays put regardless of what happens to the list behind it.
  useEffect(() => {
    quicklookRef.current = quickResult != null;
    if (quickResult) inputRef.current?.blur();
    else inputRef.current?.focus();
  }, [quickResult]);

  // Action panel and extension form are modal like Quicklook: launcher input
  // blurs while either is open.
  useEffect(() => {
    if (actionPanel || extForm) inputRef.current?.blur();
    else inputRef.current?.focus();
  }, [actionPanel, extForm]);

  // A reload swaps extension instances - a pinned panel or form would target
  // a stale provider, so drop them.
  useTauriListener("extensions-reloaded", () => {
    setActionPanel(null);
    setExtForm(null);
  });

  // Titles for the generic default-action row, by result kind. Kinds without
  // a meaningful plain-Enter (calc, dict, errors) get no row.
  const OPEN_TITLES: Record<string, string> = {
    app: "Launch",
    file: "Open",
    folder: "Open",
    command: "Run",
    "content-hint": "Search Contents",
    "content-disabled": "Open Settings",
    "ext-error": "Open Logs",
  };

  // Everything the action panel shows for the pinned result: the generic
  // default row, badge-only rows mirroring the built-in chords (Quick Look,
  // match highlight), provider-declared actions, then the global command
  // catalog. Descriptors are the single source for both rows and badges;
  // built-in badges come from the effective (possibly remapped) chords.
  const keybindSnap = useKeybinds();
  const panelActions = useMemo<ActionDescriptor[]>(() => {
    if (!actionPanel) return [];
    const result = actionPanel.result;
    const builtinBadge = (id: string) => keybindSnap.builtinShortcuts.get(id)?.[0];
    const acts: ActionDescriptor[] = [];
    if (result) {
      const pinAction: ActionDescriptor | null = canPin(result)
        ? {
            id: "app:pin",
            title: result.pinned
              ? `Unpin from “${queryRef.current.trim()}”`
              : `Pin for “${queryRef.current.trim()}”`,
            section: "result",
            shortcut: builtinBadge("builtin:pin"),
            displayOnly: true,
            run: () => togglePin(result),
          }
        : null;
      const isExt = result.id.startsWith("ext:") && !!result.ext;
      if (isExt) {
        // Extension actions lead with their own default (Enter-badged) action.
        acts.push(...collectResultActions(result, makeCtx()));
        if (pinAction) acts.push(pinAction);
      } else {
        const openTitle = OPEN_TITLES[result.kind];
        if (openTitle) {
          acts.push({
            id: "app:open",
            title: openTitle,
            section: "result",
            shortcut: { key: "enter" },
            displayOnly: true,
            run: () => { launch(result); if (quickResult) setQuickResult(null); },
          });
        }
        if (isPreviewable(result)) {
          acts.push({
            id: "app:quicklook",
            title: quickResult ? "Close Quick Look" : "Quick Look",
            section: "result",
            shortcut: builtinBadge("builtin:quick-look"),
            displayOnly: true,
            run: () => toggleQuickLook(result),
          });
        }
        if (inContents && result.title.toLowerCase().endsWith(".pdf")) {
          acts.push({
            id: "app:highlight",
            title: highlight ? "Hide Match Highlights" : "Show Match Highlights",
            section: "result",
            shortcut: builtinBadge("builtin:highlight"),
            displayOnly: true,
            run: () => setHighlight(h => !h),
          });
        }
        if (pinAction) acts.push(pinAction);
        acts.push(...collectResultActions(result, makeCtx()));
      }
    }
    return acts;
  }, [actionPanel, quickResult, highlight, inContents, keybindSnap]); // eslint-disable-line react-hooks/exhaustive-deps

  const calcResult = results.find(r => r.kind === "calc");
  // Whether there's an actual term to search (drives the empty state). Content
  // mode needs >= 2 chars; below that the list stays blank instead of "No results".
  const hasSearchTerm = inContents
    ? query.trim().length >= 2
    : browsing || query.trim().length > 0;

  // Terms to highlight in the preview - only in content (full-text) mode.
  // Outside content mode return the shared frozen EMPTY_TERMS (stable identity)
  // so every keystroke doesn't hand the preview subtree a fresh [] and churn it.
  const previewTerms = useMemo(
    () => (inContents ? deriveContentTerms(query) : EMPTY_TERMS),
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
            invoke<Config>("get_config").then(cfg => setContentEnabled(cfg.content.enabled));
            inputRef.current?.focus();
          }}
        />
      )}
      <div
        className="card"
        onMouseDown={e => {
          // Keep focus (and all keybinds) on the search input: no mousedown may
          // move focus or start a native selection. Text selection in previews
          // is re-implemented by the virtual selection engine (src/selection/).
          const t = e.target as HTMLElement;
          if (t !== inputRef.current) e.preventDefault();
        }}
      >
        <SelectionLayer actions={{ onSearch: searchFromSelection, onDefine: defineFromSelection }} />
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

        {extForm && !inClipboard && (
          <ExtensionFormModal
            form={extForm.form}
            busy={formBusy}
            onSubmit={values => activateExtension({
              id: extForm.id,
              ext: extForm.ext,
              action: extForm.form.submitAction,
              command: extForm.command,
              formValues: values,
            })}
            onClose={() => setExtForm(null)}
          />
        )}

        {activatePending && !extForm && (
          <div className="activate-pending">
            <span className="result-pending-spinner" />
            Working…
          </div>
        )}

        {toasts.length > 0 && (
          <div className="extension-toast-stack">
            {toasts.map(t => (
              <div
                key={t.id}
                className={`extension-toast ${t.level}`}
                onClick={() => setToasts(ts => ts.filter(x => x.id !== t.id))}
              >
                {t.message}
              </div>
            ))}
          </div>
        )}

        <div className="footer">
          {actionPanel && !inClipboard && (
            <ActionPanel
              actions={panelActions}
              onRun={a => { setActionPanel(null); a.run(makeCtx()); }}
              onClose={() => setActionPanel(null)}
            />
          )}
          <FooterHints selected={selected} quicklookOpen={quickResult != null} clipboardMode={inClipboard} contentMode={inContents} smartPaste={clipCaps.smart_paste} clipboardIdle={inClipboard && query.trim() === ""} pdfHighlight={highlight} actionPanelOpen={actionPanel != null} />
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
          </div>
        </div>
        </div>
      </div>
    </div>
    </ColoredIconsContext.Provider>
  );
}
