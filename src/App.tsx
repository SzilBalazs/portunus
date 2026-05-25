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
}

function ResultIcon({
  icon_path,
  title,
}: {
  icon_path?: string;
  title: string;
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

  return <div className="result-icon">{title[0]}</div>;
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

  // Focus input when window is shown (state is reset on hide instead)
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

  // Debounced search
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

  // Reset selection when results change
  useEffect(() => {
    setSelectedIndex(0);
  }, [results]);

  // Keyboard navigation
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, Math.max(results.length - 1, 0)));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        const exec = results[selectedIndex]?.exec;
        if (exec) {
          setQuery("");
          setResults([]);
          invoke("launch_app", { exec });
        }
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

  return (
    <div className="launcher" ref={containerRef}>
      <div className="card">
        <div className="search-bar">
          <svg
            className="search-icon"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.2"
          >
            <circle cx="11" cy="11" r="8" />
            <path d="m21 21-4.35-4.35" />
          </svg>
          <input
            ref={inputRef}
            className="search-input"
            type="text"
            placeholder={loading ? "Loading…" : "Search…"}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            autoFocus
          />
        </div>

        {results.length > 0 && (
          <div className="results">
            {results.map((result, i) => (
              <div
                key={result.id}
                className={`result-item${i === selectedIndex ? " selected" : ""}`}
                onMouseEnter={() => setSelectedIndex(i)}
              >
                <ResultIcon icon_path={result.icon_path} title={result.title} />
                <div className="result-text">
                  <div className="result-title">{result.title}</div>
                  {result.subtitle && (
                    <div className="result-subtitle">{result.subtitle}</div>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

export default App;
