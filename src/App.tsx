import { useState, useEffect, useLayoutEffect, useRef, useMemo, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";
import { Config, SearchResult, ClipboardCapabilities } from "./types";
import ClipboardMode from "./components/clipboard/ClipboardMode";
import { applyTheme, injectMatugenTheme } from "./theme";
import ResultsList from "./components/ResultsList";
import PreviewPanel from "./components/PreviewPanel";
import QuickLook from "./components/QuickLook";
import { deriveContentTerms } from "./highlight";
import FooterHints from "./components/FooterHints";
import { pdfView } from "./components/FilePreview";
import { dispatchLaunch, dispatchKeyDown, type LaunchContext } from "./providers/registry";
import { useTauriListener } from "./hooks/useTauriListener";
import { useIconAccents } from "./hooks/useIconAccents";
import OnboardingWizard from "./components/onboarding/OnboardingWizard";
import "./providers";
import "./App.css";
import "./themes.css";

const NON_INDEXABLE_KINDS = new Set(['calc', 'dict', 'dict-hint', 'content-hint', 'content-disabled']);

// Kinds whose preview is worth enlarging into the full-card Quicklook overlay.
const QUICKLOOK_KINDS = new Set(['file', 'folder']);

// Greyed-out completion shown after a partial command word (e.g. "tim" -> "er").
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
  // True between scheduling a search and its results arriving, so the results
  // list can distinguish "still loading" from a genuine zero-result query.
  const [searching, setSearching] = useState(false);
  // True only once a non-empty query has actually resolved with zero results.
  // Gates the content-search hint so it never flashes on the first keystroke
  // (stale empty results + pending debounce) yet stays mounted across re-searches.
  const [resolvedEmpty, setResolvedEmpty] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(0);
  // The pinned result shown in Quicklook (null = closed). Pinning the result -
  // rather than tracking a boolean + selectedIndex - keeps the overlay on the
  // same file even if background events reorder/extend the results underneath.
  const [quickResult, setQuickResult] = useState<SearchResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [version, setVersion] = useState("");
  const [indexingProgress, setIndexingProgress] = useState<{ indexed: number; total: number } | null>(null);
  const [contentEnabled, setContentEnabled] = useState(true);
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

  useEffect(() => { queryRef.current = query; }, [query]);
  useEffect(() => { getVersion().then(setVersion); }, []);

  // Load config on mount to apply theme and read content.enabled; re-apply theme on settings changes.
  useEffect(() => {
    invoke<Config>("get_config").then(cfg => {
      applyTheme(cfg.appearance);
      setContentEnabled(cfg.content.enabled);
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
    setQuery("");
    setResults([]);
    inputRef.current?.focus();
    invoke<Config>("get_config").then(cfg => { setContentEnabled(cfg.content.enabled); configRef.current = cfg; });
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
    if (clipboardMode) return;
    const m = /^(clip|clipb|clipbo|clipboa|clipboar|clipboard)\s+(.*)$/i.exec(query);
    if (m) enterClipboardMode(m[2], false);
  }, [query, clipboardMode]);

  useEffect(() => {
    if (clipboardMode) { setSearching(false); return; }
    if (!query.trim()) { setResults([]); setSearching(false); setResolvedEmpty(false); return; }
    let cancelled = false;
    setSearching(true);
    const t = setTimeout(() => {
      invoke<SearchResult[]>("search", { query }).then(r => {
        if (!cancelled) { setResults(r); setSearching(false); setResolvedEmpty(r.length === 0); }
      }).catch(() => {
        // Don't strand the UI in "searching": clear so the empty state can show.
        if (!cancelled) { setResults([]); setSearching(false); setResolvedEmpty(true); }
      });
    }, 10);
    return () => { cancelled = true; clearTimeout(t); };
  }, [query, clipboardMode]);

  useEffect(() => { setSelectedIndex(0); }, [query]);

  const displayResults = useMemo<SearchResult[]>(() => {
    if (!query.trim()) return [];
    const isContent = query.trimStart().startsWith('!');
    if (isContent) {
      // Strip the '!' prefix; with no term yet ("!", "!  ") there is nothing to
      // search, so show nothing rather than a stray disabled hint or empty state.
      const term = query.trimStart().slice(1).trim();
      if (!term) return [];
      if (!contentEnabled) {
        return [{
          id: "content:disabled",
          title: "Content search is disabled",
          subtitle: "Open Settings → Content to enable",
          kind: "content-disabled",
          score: 0,
        }];
      }
      return results;
    }
    if (results.length === 0) {
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
    return results;
  }, [query, results, contentEnabled, resolvedEmpty]);

  // Results can shrink without the query changing (search-invalidated). Snap the
  // selection back in bounds so the highlight/preview and Enter stay live.
  useEffect(() => {
    setSelectedIndex(i => (i >= displayResults.length ? 0 : i));
  }, [displayResults.length]);

  const requery = () => {
    const q = queryRef.current;
    if (q.trim()) invoke<SearchResult[]>("search", { query: q }).then(setResults);
  };

  useTauriListener("search-invalidated", () => {
    requery();
    // Also refresh content.enabled so the disabled hint appears/disappears
    // immediately when the user toggles content search in Settings.
    invoke<Config>("get_config").then(cfg => { setContentEnabled(cfg.content.enabled); configRef.current = cfg; });
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
    if (result.kind === "content-disabled") {
      invoke("open_settings_window", { section: "content" });
      return;
    }
    if (result.kind === "content-hint") {
      setQuery('! ' + queryRef.current.trim());
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
    invoke("launch_app", { exec, id: result.id, kind: result.kind });
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
        setSelectedIndex(i => Math.min(i + 1, Math.max(displayResults.length - 1, 0)));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex(i => Math.max(i - 1, 0));
      } else if (e.shiftKey && e.key === "Enter") {
        // Quicklook: pin & expand the selected file/folder preview to fill the card.
        // Placed before the plain-Enter launch so Enter alone still opens.
        e.preventDefault();
        if (quickResult) { setQuickResult(null); return; }
        const sel = displayResults[selectedIndex];
        if (sel && QUICKLOOK_KINDS.has(sel.kind)) setQuickResult(sel);
      } else if (e.altKey && !e.ctrlKey && !e.metaKey && e.key >= "1" && e.key <= "9") {
        e.preventDefault();
        const target = launchableResults[parseInt(e.key) - 1];
        if (!target) return;
        setSelectedIndex(displayResults.indexOf(target));
        launch(target);
      } else if (e.key === "Tab") {
        // Accept the ghost completion if one is showing; otherwise keep focus
        // in the input (no focusable peers to tab to in the launcher).
        e.preventDefault();
        const ghost = ghostRef.current;
        if (ghost) setQuery(queryRef.current + ghost);
      } else if (e.key === "Escape") {
        e.preventDefault();
        // Esc closes the Quicklook overlay first, the window second.
        if (quickResult) { setQuickResult(null); return; }
        setQuery("");
        setResults([]);
        invoke("hide_window");
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
  }, [displayResults, selectedIndex, quickResult]);

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

  const calcResult = results.find(r => r.kind === "calc");
  const isContentSearch = query.trimStart().startsWith('!');
  // Whether there's an actual term to search. For content queries that means
  // something after the '!'; a bare "!"/"!  " has no term (so no "No results").
  const hasSearchTerm = isContentSearch
    ? query.trimStart().slice(1).trim().length > 0
    : query.trim().length > 0;

  // Terms to highlight in the preview - only for content (`!`) searches.
  const previewTerms = useMemo(
    () => (isContentSearch ? deriveContentTerms(query) : []),
    [isContentSearch, query],
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
    return null;
  }, [query]);

  const contentSized = ghostSuffix !== null || hintChip !== null || calcResult != null;

  return (
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
            <span ref={mirrorRef} className="input-mirror" aria-hidden="true">{query}</span>
            <input
              ref={inputRef}
              className="search-input"
              type="text"
              placeholder={clipboardMode ? "Search clipboard history…" : (loading ? "Loading…" : "Search…")}
              value={query}
              onChange={e => setQuery(e.target.value)}
              autoFocus
              spellCheck={false}
              style={!clipboardMode && contentSized ? { width: inputWidth + 4 } : { flex: 1 }}
            />
            {!clipboardMode && ghostSuffix && <span className="search-ghost">{ghostSuffix}</span>}
            {!clipboardMode && hintChip && <span className="search-hint-chip">{hintChip}</span>}
            {!clipboardMode && calcResult && <div className="calc-inline">= {calcResult.title}</div>}
            {!clipboardMode && contentSized && <div className="search-spacer" />}
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
            onSelect={setSelectedIndex}
            onLaunch={launch}
            launchableResults={launchableResults}
            accents={accents}
          />
          <div className="preview-col">
            <PreviewPanel
              result={selected}
              onLaunch={() => launch(selected ?? undefined)}
              onReveal={() => { setQuery(""); setResults([]); }}
              terms={previewTerms}
            />
          </div>
        </div>
        )}

        {quickResult && !clipboardMode && (
          <QuickLook
            result={quickResult}
            terms={previewTerms}
            onLaunch={() => { launch(quickResult); setQuickResult(null); }}
            onClose={() => setQuickResult(null)}
          />
        )}

        <div className="footer">
          <FooterHints selected={selected} canComplete={ghostSuffix !== null} quicklookOpen={quickResult != null} clipboardMode={clipboardMode} smartPaste={clipCaps.smart_paste} clipboardIdle={clipboardMode && query.trim() === ""} />
          <div className="footer-right">
            <button
              className="footer-settings-btn"
              onClick={() => {
                // Leave clipboard mode / clear the query so the launcher is clean
                // when it's next shown over (or instead of) the settings window.
                setClipboardMode(false);
                setQuery("");
                setResults([]);
                invoke("open_settings_window");
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
  );
}
