import { useState, useEffect, useLayoutEffect, useRef, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { SearchResult, ExpiredTimer } from "./types";
import { playTimerChime, audioCtxWarmup } from "./utils";
import ResultsList from "./components/ResultsList";
import PreviewPanel from "./components/PreviewPanel";
import FooterHints from "./components/FooterHints";
import { dispatchLaunch, dispatchKeyDown, type LaunchContext } from "./providers/registry";
import "./providers";
import "./App.css";

const NON_INDEXABLE_KINDS = new Set(['calc', 'dict', 'dict-hint', 'timer-hint', 'content-hint']);

export default function App() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [loading, setLoading] = useState(true);
  const [expiredTimers, setExpiredTimers] = useState<ExpiredTimer[]>([]);
  const [version, setVersion] = useState("");
  const [indexingProgress, setIndexingProgress] = useState<{ indexed: number; total: number } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const mirrorRef = useRef<HTMLSpanElement>(null);
  const [inputWidth, setInputWidth] = useState(0);
  const queryRef = useRef(query);

  useEffect(() => { queryRef.current = query; }, [query]);
  useEffect(() => { getVersion().then(setVersion); }, []);

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

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    listen("window-show", () => {
      inputRef.current?.focus();
      audioCtxWarmup();
    }).then(fn => {
      if (active) unlisten = fn; else fn();
    });
    return () => { active = false; unlisten?.(); };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    listen<string>("window-show-query", event => {
      setQuery(event.payload);
      // Don't manually clear results — the query useEffect will re-search immediately.
      inputRef.current?.focus();
      audioCtxWarmup();
    }).then(fn => {
      if (active) unlisten = fn; else fn();
    });
    return () => { active = false; unlisten?.(); };
  }, []);

  useEffect(() => {
    if (!query.trim()) { setResults([]); return; }
    let cancelled = false;
    const t = setTimeout(() => {
      invoke<SearchResult[]>("search", { query }).then(r => { if (!cancelled) setResults(r); });
    }, 40);
    return () => { cancelled = true; clearTimeout(t); };
  }, [query]);

  useEffect(() => { setSelectedIndex(0); }, [query]);

  const displayResults = useMemo<SearchResult[]>(() => {
    if (query.trim()) {
      const isContent = query.trimStart().startsWith('!');
      if (results.length === 0 && !isContent) {
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
  }, [query, results, expiredTimers]);

  const hasTimerItems = displayResults.some(r => r.kind === "timer-item");
  useEffect(() => {
    if (!hasTimerItems) return;
    const id = setInterval(() => {
      const q = queryRef.current;
      if (q.trim()) invoke<SearchResult[]>("search", { query: q }).then(setResults);
    }, 1000);
    return () => clearInterval(id);
  }, [hasTimerItems]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    listen<ExpiredTimer>("timer-expired", event => {
      playTimerChime();
      setExpiredTimers(prev => [...prev, event.payload]);
      setQuery("");
    }).then(fn => { if (active) unlisten = fn; else fn(); });
    return () => { active = false; unlisten?.(); };
  }, []);

  const requery = () => {
    const q = queryRef.current;
    if (q.trim()) invoke<SearchResult[]>("search", { query: q }).then(setResults);
  };

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    listen("search-invalidated", requery).then(fn => {
      if (active) unlisten = fn; else fn();
    });
    return () => { active = false; unlisten?.(); };
  }, []);

  const makeCtx = (): LaunchContext => ({
    setQuery,
    setResults,
    requery,
    removeExpiredTimer: (id: number) => setExpiredTimers(prev => prev.filter(t => t.id !== id)),
  });

  const launch = (result?: SearchResult) => {
    if (!result) return;
    if (result.kind === "content-hint") {
      setQuery('! ' + queryRef.current.trim());
      return;
    }
    const ctx = makeCtx();
    if (dispatchLaunch(result, ctx)) return;
    if (!result.exec) return;
    setQuery("");
    setResults([]);
    invoke("launch_app", { exec: result.exec, id: result.id, kind: result.kind });
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
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
        e.preventDefault();
        const q = queryRef.current;
        if (/^!\s*/.test(q)) {
          setQuery(q.replace(/^!\s*/, ''));
        } else {
          setQuery('! ' + q.trim());
        }
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

  const launchableResults = useMemo(
    () => displayResults.filter(r => !NON_INDEXABLE_KINDS.has(r.kind)),
    [displayResults]
  );

  const ghostSuffix = useMemo((): string | null => {
    const q = query;
    if (q.length < 2 || q.includes(' ')) return null;
    if ('timer'.startsWith(q) && q.length < 5) return 'timer'.slice(q.length);
    if ('define'.startsWith(q) && q.length < 6) return 'define'.slice(q.length);
    if ('dict'.startsWith(q) && q.length < 4 && !'define'.startsWith(q)) return 'dict'.slice(q.length);
    if ('clipboard'.startsWith(q) && q.length >= 4 && q.length < 9) return 'clipboard'.slice(q.length);
    return null;
  }, [query]);

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
            query={query}
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
            />
          </div>
        </div>

        <div className="footer">
          <FooterHints selected={selected} isContentSearch={isContentSearch} />
          <div className="brand">Portunus{version && <span className="brand-version">v{version}</span>}</div>
        </div>
      </div>
    </div>
  );
}
