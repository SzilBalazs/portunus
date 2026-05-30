import { useState, useEffect, useLayoutEffect, useRef, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";
import { Config, SearchResult, ExpiredTimer } from "./types";
import { playTimerChime, audioCtxWarmup } from "./utils";
import { applyTheme } from "./theme";
import ResultsList from "./components/ResultsList";
import PreviewPanel from "./components/PreviewPanel";
import { deriveContentTerms } from "./highlight";
import FooterHints from "./components/FooterHints";
import { pdfView } from "./components/FilePreview";
import { dispatchLaunch, dispatchKeyDown, type LaunchContext } from "./providers/registry";
import { useTauriListener } from "./hooks/useTauriListener";
import OnboardingWizard from "./components/onboarding/OnboardingWizard";
import "./providers";
import "./App.css";
import "./themes.css";

const NON_INDEXABLE_KINDS = new Set(['calc', 'dict', 'dict-hint', 'timer-hint', 'content-hint', 'content-disabled']);

// Greyed-out completion shown after a partial command word (e.g. "tim" -> "er").
// Tab accepts it. Returns the suffix to append, or null when nothing completes.
function ghostFor(q: string): string | null {
  if (q.length < 2 || q.includes(' ')) return null;
  if ('timer'.startsWith(q) && q.length < 5) return 'timer'.slice(q.length);
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
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [loading, setLoading] = useState(true);
  const [expiredTimers, setExpiredTimers] = useState<ExpiredTimer[]>([]);
  const [version, setVersion] = useState("");
  const [indexingProgress, setIndexingProgress] = useState<{ indexed: number; total: number } | null>(null);
  const [contentEnabled, setContentEnabled] = useState(true);
  const [onboardConfig, setOnboardConfig] = useState<Config | null>(null);
  const [showOnboarding, setShowOnboarding] = useState(false);
  const showOnboardingRef = useRef(false);
  useEffect(() => { showOnboardingRef.current = showOnboarding; }, [showOnboarding]);
  const inputRef = useRef<HTMLInputElement>(null);
  const mirrorRef = useRef<HTMLSpanElement>(null);
  const [inputWidth, setInputWidth] = useState(0);
  const queryRef = useRef(query);

  useEffect(() => { queryRef.current = query; }, [query]);
  useEffect(() => { getVersion().then(setVersion); }, []);

  // Load config on mount to apply theme and read content.enabled; re-apply theme on settings changes.
  useEffect(() => {
    invoke<Config>("get_config").then(cfg => {
      applyTheme(cfg.appearance);
      setContentEnabled(cfg.content.enabled);
      setOnboardConfig(cfg);
      if (!cfg.general.onboarding_completed) setShowOnboarding(true);
    });
    let unlisten: (() => void) | undefined;
    listen<Config["appearance"]>("appearance-changed", event => {
      applyTheme(event.payload);
    }).then(fn => { unlisten = fn; });
    return () => { unlisten?.(); };
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
    inputRef.current?.focus();
    audioCtxWarmup();
    invoke<Config>("get_config").then(cfg => setContentEnabled(cfg.content.enabled));
  });

  useEffect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    win.onFocusChanged(({ payload: focused }) => {
      if (focused) inputRef.current?.focus();
    }).then(fn => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);

  useTauriListener<string>("window-show-query", payload => {
    setQuery(payload);
    // Don't manually clear results — the query useEffect will re-search immediately.
    inputRef.current?.focus();
    audioCtxWarmup();
  });

  useEffect(() => {
    if (!query.trim()) { setResults([]); setSearching(false); return; }
    let cancelled = false;
    setSearching(true);
    const t = setTimeout(() => {
      invoke<SearchResult[]>("search", { query }).then(r => {
        if (!cancelled) { setResults(r); setSearching(false); }
      }).catch(() => {
        // Don't strand the UI in "searching": clear so the empty state can show.
        if (!cancelled) { setResults([]); setSearching(false); }
      });
    }, 40);
    return () => { cancelled = true; clearTimeout(t); };
  }, [query]);

  useEffect(() => { setSelectedIndex(0); }, [query]);

  const displayResults = useMemo<SearchResult[]>(() => {
    if (query.trim()) {
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
        // Suggest content search only once the regular search has resolved with
        // nothing — otherwise the hint flashes before the first debounce lands.
        if (searching) return [];
        return [{
          id: "content:hint",
          title: "Search file contents",
          subtitle: `Search for "${query.trim()}" inside files`,
          kind: "content-hint",
          score: 0,
        }];
      }
      return results;
    }
    if (expiredTimers.length === 0) return [];
    return expiredTimers.map(t => ({
      id: `timer:expired:${t.id}`,
      title: t.label,
      subtitle: "Timer finished",
      kind: "timer-expired",
      score: 0,
      exec: `timer:dismiss:${t.id}`,
    }));
  }, [query, results, expiredTimers, contentEnabled, searching]);

  // Results can shrink without the query changing (timer requery, search-invalidated).
  // Snap the selection back in bounds so the highlight/preview and Enter stay live.
  useEffect(() => {
    setSelectedIndex(i => (i >= displayResults.length ? 0 : i));
  }, [displayResults.length]);

  const hasTimerItems = displayResults.some(r => r.kind === "timer-item");
  useEffect(() => {
    if (!hasTimerItems) return;
    const id = setInterval(() => {
      const q = queryRef.current;
      if (q.trim()) invoke<SearchResult[]>("search", { query: q }).then(setResults);
    }, 1000);
    return () => clearInterval(id);
  }, [hasTimerItems]);

  useTauriListener<ExpiredTimer>("timer-expired", payload => {
    playTimerChime();
    setExpiredTimers(prev => [...prev, payload]);
    setQuery("");
  });

  const requery = () => {
    const q = queryRef.current;
    if (q.trim()) invoke<SearchResult[]>("search", { query: q }).then(setResults);
  };

  useTauriListener("search-invalidated", () => {
    requery();
    // Also refresh content.enabled so the disabled hint appears/disappears
    // immediately when the user toggles content search in Settings.
    invoke<Config>("get_config").then(cfg => setContentEnabled(cfg.content.enabled));
  });

  const makeCtx = (): LaunchContext => ({
    setQuery,
    setResults,
    requery,
    removeExpiredTimer: (id: number) => setExpiredTimers(prev => prev.filter(t => t.id !== id)),
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
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex(i => Math.min(i + 1, Math.max(displayResults.length - 1, 0)));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex(i => Math.max(i - 1, 0));
      } else if (e.altKey && !e.ctrlKey && !e.metaKey && e.key >= "1" && e.key <= "9") {
        e.preventDefault();
        const target = launchableResults[parseInt(e.key) - 1];
        if (!target) return;
        setSelectedIndex(displayResults.indexOf(target));
        if (target.kind !== "timer-item") launch(target);
      } else if (e.key === "Tab") {
        // Accept the ghost completion if one is showing; otherwise keep focus
        // in the input (no focusable peers to tab to in the launcher).
        e.preventDefault();
        const q = queryRef.current;
        const ghost = ghostFor(q);
        if (ghost) setQuery(q + ghost);
      } else if (e.key === "Escape") {
        e.preventDefault();
        setQuery("");
        setResults([]);
        invoke("hide_window");
      } else {
        const ctx = makeCtx();
        if (!dispatchKeyDown(e, selected, ctx)) {
          if (e.key === "Enter") launch(displayResults[selectedIndex]);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [displayResults, selectedIndex]);

  const selected = displayResults[selectedIndex] ?? null;
  const calcResult = results.find(r => r.kind === "calc");
  const isContentSearch = query.trimStart().startsWith('!');
  // Whether there's an actual term to search. For content queries that means
  // something after the '!'; a bare "!"/"!  " has no term (so no "No results").
  const hasSearchTerm = isContentSearch
    ? query.trimStart().slice(1).trim().length > 0
    : query.trim().length > 0;

  // Terms to highlight in the preview — only for content (`!`) searches.
  const previewTerms = useMemo(
    () => (isContentSearch ? deriveContentTerms(query) : []),
    [isContentSearch, query],
  );

  const launchableResults = useMemo(
    () => displayResults.filter(r => !NON_INDEXABLE_KINDS.has(r.kind)),
    [displayResults]
  );

  const ghostSuffix = useMemo(() => ghostFor(query), [query]);

  const hintChip = useMemo((): string | null => {
    const q = query;
    if (q === 'timer' || q === 'timer ') return '<duration> [label]';
    if (q === 'define' || q === 'define ') return '<word>';
    if (q === 'dict' || q === 'dict ') return '<word>';
    if (q === 'clipboard' || q === 'clipboard ') return '<search>';
    return null;
  }, [query]);

  const contentSized = ghostSuffix !== null || hintChip !== null || calcResult != null;

  const stopSelectedTimer = () => {
    const sel = displayResults[selectedIndex];
    if (sel?.kind === "timer-item" && sel.exec) launch(sel);
  };

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
      <div className="card" onMouseDown={e => {
        const t = e.target as HTMLElement;
        if (t !== inputRef.current && !t.closest('pre, code')) e.preventDefault();
      }}>
        <div className="search-bar">
          {indexingProgress && (
            <div className="content-index-bar">
              <div
                className="content-index-fill"
                style={{ width: `${Math.min(100, (indexingProgress.indexed / Math.max(1, indexingProgress.total)) * 100)}%` }}
              />
            </div>
          )}
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
          <div className="search-input-area">
            <span ref={mirrorRef} className="input-mirror" aria-hidden="true">{query}</span>
            <input
              ref={inputRef}
              className="search-input"
              type="text"
              placeholder={loading ? "Loading…" : "Search…"}
              value={query}
              onChange={e => setQuery(e.target.value)}
              autoFocus
              spellCheck={false}
              style={contentSized ? { width: inputWidth + 4 } : { flex: 1 }}
            />
            {ghostSuffix && <span className="search-ghost">{ghostSuffix}</span>}
            {hintChip && <span className="search-hint-chip">{hintChip}</span>}
            {calcResult && <div className="calc-inline">= {calcResult.title}</div>}
            <div className="search-spacer" />
          </div>
        </div>

        <div className="body">
          <ResultsList
            results={displayResults}
            selectedIndex={selectedIndex}
            active={hasSearchTerm}
            searching={searching}
            onSelect={setSelectedIndex}
            onLaunch={launch}
            launchableResults={launchableResults}
          />
          <div className="preview-col">
            <PreviewPanel
              result={selected}
              onLaunch={() => launch(selected ?? undefined)}
              onStopTimer={stopSelectedTimer}
              onReveal={() => { setQuery(""); setResults([]); }}
              terms={previewTerms}
            />
          </div>
        </div>

        <div className="footer">
          <FooterHints selected={selected} canComplete={ghostSuffix !== null} />
          <div className="footer-right">
            <button
              className="footer-settings-btn"
              onClick={() => invoke("open_settings_window")}
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
  );
}
