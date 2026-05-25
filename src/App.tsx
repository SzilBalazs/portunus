import { useState, useEffect, useRef } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

interface SearchResult {
  id: string;
  title: string;
  subtitle?: string;
  kind: string;
  score: number;
  exec?: string;
  icon_path?: string;
  file_size?: number;
  created?: number;
  modified?: number;
}

function ResultIcon({ icon_path, title, kind }: {
  icon_path?: string;
  title: string;
  kind: string;
}) {
  const [failed, setFailed] = useState(false);

  if (icon_path && !failed) {
    return (
      <img
        className="result-icon-img"
        src={convertFileSrc(icon_path)}
        alt=""
        onError={() => setFailed(true)}
      />
    );
  }

  if (kind === "calc") {
    return (
      <div className="result-icon">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="16" height="16">
          <rect x="4" y="2" width="16" height="20" rx="2" />
          <rect x="7" y="5" width="10" height="4" rx="1" />
          <circle cx="8" cy="14" r="1" fill="currentColor" stroke="none" />
          <circle cx="12" cy="14" r="1" fill="currentColor" stroke="none" />
          <circle cx="16" cy="14" r="1" fill="currentColor" stroke="none" />
          <circle cx="8" cy="18" r="1" fill="currentColor" stroke="none" />
          <circle cx="12" cy="18" r="1" fill="currentColor" stroke="none" />
          <circle cx="16" cy="18" r="1" fill="currentColor" stroke="none" />
        </svg>
      </div>
    );
  }

  if (kind === "folder") {
    return (
      <div className="result-icon">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="16" height="16">
          <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
        </svg>
      </div>
    );
  }

  if (kind === "file") {
    return (
      <div className="result-icon">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="16" height="16">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
          <polyline points="14 2 14 8 20 8" />
        </svg>
      </div>
    );
  }

  return <div className="result-icon">{title[0]}</div>;
}

function AppPreview({ result, onLaunch }: { result: SearchResult; onLaunch: () => void }) {
  const [iconFailed, setIconFailed] = useState(false);

  return (
    <div className="app-preview">
      <div className="app-preview-hero">
        {result.icon_path && !iconFailed ? (
          <img
            className="app-preview-icon"
            src={convertFileSrc(result.icon_path)}
            alt=""
            onError={() => setIconFailed(true)}
          />
        ) : (
          <div className="app-preview-icon-fallback">{result.title[0]}</div>
        )}
        <div>
          <div className="app-preview-name">{result.title}</div>
          {result.subtitle && <div className="app-preview-sub">{result.subtitle}</div>}
        </div>
      </div>
      <div className="app-preview-actions">
        <button className="btn-primary" onClick={onLaunch}>
          Launch <span className="btn-kbd">↵</span>
        </button>
      </div>
    </div>
  );
}

function PreviewPanel({ result, onLaunch }: { result: SearchResult | null; onLaunch: () => void }) {
  if (result?.kind === "app") {
    return <AppPreview result={result} onLaunch={onLaunch} />;
  }
  return <div className="preview-empty" />;
}

function App() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [loading, setLoading] = useState(true);
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let done = false;
    const markReady = () => { if (!done) { done = true; setLoading(false); } };
    const promise = listen("apps-ready", markReady);
    invoke<boolean>("is_apps_ready").then(ready => { if (ready) markReady(); });
    return () => {
      done = true;
      promise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    listen("window-show", () => {
      inputRef.current?.focus();
    }).then((fn) => {
      if (active) unlisten = fn;
      else fn();
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!query.trim()) {
      setResults([]);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      invoke<SearchResult[]>("search", { query }).then((r) => {
        if (!cancelled) setResults(r);
      });
    }, 40);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query]);

  useEffect(() => {
    setSelectedIndex(0);
  }, [results]);

  const launch = (exec?: string) => {
    if (exec) {
      setQuery("");
      setResults([]);
      invoke("launch_app", { exec });
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, Math.max(results.length - 1, 0)));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        launch(results[selectedIndex]?.exec);
      } else if (e.key === "Escape") {
        e.preventDefault();
        setQuery("");
        setResults([]);
        invoke("hide_window");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [results, selectedIndex]);

  const selected = results[selectedIndex] ?? null;
  const calcResult = results.find((r) => r.kind === "calc");

  return (
    <div className="launcher" ref={containerRef}>
      <div className="card">
        <div className="search-bar">
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
          <input
            ref={inputRef}
            className="search-input"
            type="text"
            placeholder={loading ? "Loading…" : "Search apps…"}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            autoFocus
            spellCheck={false}
          />
          {calcResult && (
            <div className="calc-inline">= {calcResult.title}</div>
          )}
        </div>

        <div className="body">
          <div className="results-col" role="listbox">
            {query && results.length === 0 && (
              <div className="results-empty">No results</div>
            )}
            {results.map((result, i) => (
              <div
                key={result.id}
                className={`result-row${i === selectedIndex ? " selected" : ""}`}
                role="option"
                aria-selected={i === selectedIndex}
                onMouseEnter={() => setSelectedIndex(i)}
                onClick={() => launch(result.exec)}
              >
                <ResultIcon icon_path={result.icon_path} title={result.title} kind={result.kind} />
                <div className="result-text">
                  <div className="result-title">{result.title}</div>
                  {result.subtitle && (
                    <div className="result-subtitle">{result.subtitle}</div>
                  )}
                </div>
              </div>
            ))}
          </div>

          <div className="preview-col">
            <PreviewPanel result={selected} onLaunch={() => launch(selected?.exec)} />
          </div>
        </div>

        <div className="footer">
          <div className="hints">
            <span className="hint"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
            <span className="hint"><kbd>↵</kbd> open</span>
            <span className="hint"><kbd>Esc</kbd> close</span>
          </div>
          <div className="brand">Portunus</div>
        </div>
      </div>
    </div>
  );
}

export default App;
