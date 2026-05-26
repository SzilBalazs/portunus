import { useState, useEffect, useRef, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { SearchResult, ExpiredTimer } from "./types";
import { playTimerChime, audioCtxWarmup } from "./utils";
import ResultsList from "./components/ResultsList";
import PreviewPanel from "./components/PreviewPanel";
import FooterHints from "./components/FooterHints";
import "./App.css";

export default function App() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [loading, setLoading] = useState(true);
  const [expiredTimers, setExpiredTimers] = useState<ExpiredTimer[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);
  const queryRef = useRef(query);

  useEffect(() => { queryRef.current = query; }, [query]);

  // Wait for the background provider thread to finish loading apps.
  useEffect(() => {
    let done = false;
    const markReady = () => { if (!done) { done = true; setLoading(false); } };
    const promise = listen("apps-ready", markReady);
    invoke<boolean>("is_apps_ready").then(ready => { if (ready) markReady(); });
    return () => { done = true; promise.then(ul => ul()); };
  }, []);

  // Re-focus the input whenever the window is shown via IPC.
  // Also warm up the AudioContext on first show so it isn't suspended when a timer fires.
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

  // Handle show-with-initial-query (e.g. --clipboard flag).
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    listen<string>("window-show-query", event => {
      setQuery(event.payload);
      setResults([]);
      inputRef.current?.focus();
      audioCtxWarmup();
    }).then(fn => {
      if (active) unlisten = fn; else fn();
    });
    return () => { active = false; unlisten?.(); };
  }, []);

  // Debounced search: fire 40ms after the last query change.
  useEffect(() => {
    if (!query.trim()) { setResults([]); return; }
    let cancelled = false;
    const t = setTimeout(() => {
      invoke<SearchResult[]>("search", { query }).then(r => { if (!cancelled) setResults(r); });
    }, 40);
    return () => { cancelled = true; clearTimeout(t); };
  }, [query]);

  // Reset selection on every new query, but NOT on timer auto-refresh.
  useEffect(() => { setSelectedIndex(0); }, [query]);

  // When query is empty, overlay unacknowledged expired timers as results.
  const displayResults = useMemo<SearchResult[]>(() => {
    if (query.trim()) return results;
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

  // Re-query every second while running timers are visible to keep countdowns live.
  const hasTimerItems = displayResults.some(r => r.kind === "timer-item");
  useEffect(() => {
    if (!hasTimerItems) return;
    const id = setInterval(() => {
      const q = queryRef.current;
      if (q.trim()) invoke<SearchResult[]>("search", { query: q }).then(setResults);
    }, 1000);
    return () => clearInterval(id);
  }, [hasTimerItems]);

  // Show expired timers when the backend signals one finished.
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

  const launch = (result?: SearchResult) => {
    const exec = result?.exec;
    if (!exec) return;

    if (exec.startsWith("clipboard:copy:")) {
      invoke("paste_clipboard", { id: exec.slice("clipboard:copy:".length) });
      setQuery("");
      setResults([]);
      return;
    }

    if (exec.startsWith("timer:create:")) {
      const rest = exec.slice("timer:create:".length);
      const colon = rest.indexOf(":");
      invoke("create_timer", { durationSecs: parseInt(rest.slice(0, colon)), label: rest.slice(colon + 1) });
      setQuery("timer");
      setResults([]);
      return;
    }
    if (exec.startsWith("timer:stop:")) {
      invoke("stop_timer", { id: parseInt(exec.slice("timer:stop:".length)) });
      requery();
      return;
    }
    if (exec.startsWith("timer:dismiss:")) {
      const id = parseInt(exec.slice("timer:dismiss:".length));
      setExpiredTimers(prev => prev.filter(t => t.id !== id));
      return;
    }

    setQuery("");
    setResults([]);
    invoke("launch_app", { exec, id: result?.id, kind: result?.kind });
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex(i => Math.min(i + 1, Math.max(displayResults.length - 1, 0)));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex(i => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        const sel = displayResults[selectedIndex];
        // Don't stop a running timer accidentally with Enter — require Del for that.
        if (sel?.kind !== "timer-item") launch(sel);
      } else if (e.key === "Delete") {
        const sel = displayResults[selectedIndex];
        if (sel?.kind === "timer-item" || sel?.kind === "timer-expired") {
          e.preventDefault();
          launch(sel);
        }
      } else if (e.ctrlKey && !e.altKey && e.key === "c") {
        const sel = displayResults[selectedIndex];
        if (sel?.kind === "calc") {
          e.preventDefault();
          navigator.clipboard.writeText(sel.title);
        } else if (sel?.kind === "file" || sel?.kind === "folder") {
          e.preventDefault();
          const path = sel.subtitle ? `${sel.subtitle}/${sel.title}` : sel.title;
          navigator.clipboard.writeText(path);
        }
      } else if (e.altKey && !e.ctrlKey && !e.metaKey && e.key >= "1" && e.key <= "9") {
        e.preventDefault();
        const idx = parseInt(e.key) - 1;
        const target = displayResults[idx];
        if (!target) return;
        setSelectedIndex(idx);
        if (target.kind !== "timer-item") launch(target);
      } else if (e.key === "Escape") {
        e.preventDefault();
        setQuery("");
        setResults([]);
        invoke("hide_window");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [displayResults, selectedIndex]);

  const selected = displayResults[selectedIndex] ?? null;
  const calcResult = results.find(r => r.kind === "calc");

  const stopSelectedTimer = () => {
    const sel = displayResults[selectedIndex];
    if (sel?.kind === "timer-item" && sel.exec) launch(sel);
  };

  return (
    <div className="launcher">
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
            placeholder={loading ? "Loading…" : "Search…"}
            value={query}
            onChange={e => setQuery(e.target.value)}
            autoFocus
            spellCheck={false}
          />
          {calcResult && <div className="calc-inline">= {calcResult.title}</div>}
        </div>

        <div className="body">
          <ResultsList
            results={displayResults}
            selectedIndex={selectedIndex}
            query={query}
            onSelect={setSelectedIndex}
            onLaunch={launch}
          />
          <div className="preview-col">
            <PreviewPanel
              result={selected}
              onLaunch={() => launch(selected ?? undefined)}
              onStopTimer={stopSelectedTimer}
            />
          </div>
        </div>

        <div className="footer">
          <FooterHints selected={selected} />
          <div className="brand">Portunus</div>
        </div>
      </div>
    </div>
  );
}
