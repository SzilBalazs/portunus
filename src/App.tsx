import { Fragment, useState, useEffect, useRef } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

function shortenPath(path: string): string {
  return path.replace(/^\/home\/[^/]+/, "~").replace(/^\/root/, "~");
}

function groupLabel(kind: string): string | null {
  if (kind === "app") return "APPS";
  if (kind === "file" || kind === "folder") return "FILES";
  return null;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(1)} GB`;
}

function formatDate(ts: number): string {
  return new Date(ts * 1000).toLocaleDateString("en-US", {
    year: "numeric", month: "short", day: "numeric",
  });
}

function fileKind(title: string, isFolder: boolean): string {
  if (isFolder) return "Folder";
  const ext = title.split(".").pop()?.toLowerCase() ?? "";
  const map: Record<string, string> = {
    pdf: "PDF Document",
    png: "PNG Image", jpg: "JPEG Image", jpeg: "JPEG Image",
    gif: "GIF Image", webp: "WebP Image", svg: "SVG Image",
    ts: "TypeScript Source", tsx: "TypeScript Source",
    js: "JavaScript Source", jsx: "JavaScript Source",
    rs: "Rust Source", py: "Python Source", go: "Go Source",
    md: "Markdown", txt: "Text File",
    zip: "Archive", tar: "Archive", gz: "Archive",
    bz2: "Archive", xz: "Archive", "7z": "Archive", rar: "Archive",
    mp4: "Video", mkv: "Video", mov: "Video", avi: "Video",
    mp3: "Audio", flac: "Audio", wav: "Audio", ogg: "Audio",
    json: "JSON Data", xml: "XML Document",
    html: "HTML Document", css: "CSS Stylesheet",
    sh: "Shell Script", toml: "TOML Config",
    yaml: "YAML Config", yml: "YAML Config",
  };
  return map[ext] ?? "File";
}

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

const pdfPromiseCache = new Map<string, Promise<string>>();
const pdfUrlCache = new Map<string, string>();

function getPdfUrl(path: string): Promise<string> {
  if (!pdfPromiseCache.has(path)) {
    pdfPromiseCache.set(path,
      invoke<number[]>("render_pdf_page", { path }).then((bytes) => {
        const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: "image/jpeg" }));
        pdfUrlCache.set(path, url);
        return url;
      })
    );
  }
  return pdfPromiseCache.get(path)!;
}

function PdfPreview({ path }: { path: string }) {
  const [src, setSrc] = useState<string | null>(() => pdfUrlCache.get(path) ?? null);
  const [loaded, setLoaded] = useState(() => pdfUrlCache.has(path));
  const [error, setError] = useState(false);

  useEffect(() => {
    const cached = pdfUrlCache.get(path);
    if (cached) {
      setSrc(cached);
      setLoaded(true);
      setError(false);
      return;
    }
    let cancelled = false;
    setSrc(null);
    setLoaded(false);
    setError(false);
    getPdfUrl(path)
      .then((url) => { if (!cancelled) setSrc(url); })
      .catch((e) => {
        console.error("[pdf] render_pdf_page failed:", e);
        if (!cancelled) setError(true);
      });
    return () => { cancelled = true; };
  }, [path]);

  const isLoading = !src && !error;

  return (
    <div className={`pdf-preview-wrap${isLoading ? " is-loading" : ""}`}>
      {isLoading && <div className="pdf-skeleton" />}
      {src && (
        <img
          src={src}
          alt="PDF preview"
          style={{ opacity: loaded ? 1 : 0 }}
          onLoad={() => setLoaded(true)}
        />
      )}
      {error && <span className="pdf-preview-msg">Preview unavailable</span>}
    </div>
  );
}

function FilePreview({ result }: { result: SearchResult }) {
  const isFolder = result.kind === "folder";
  const kind = fileKind(result.title, isFolder);
  const tag = [kind, !isFolder && result.file_size != null ? formatBytes(result.file_size) : null]
    .filter(Boolean).join(" · ");
  const filePath = result.subtitle ? `${result.subtitle}/${result.title}` : result.title;
  const isPdf = kind === "PDF Document";

  const icon = isFolder ? (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="22" height="22">
      <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
    </svg>
  ) : (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="22" height="22">
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
      <polyline points="14 2 14 8 20 8" />
    </svg>
  );

  return (
    <div className="file-preview">
      <div className="file-preview-head">
        <div className="file-preview-icon-wrap">{icon}</div>
        <div className="file-preview-head-text">
          <div className="file-preview-title">{result.title}</div>
          <div className="file-preview-tag">{tag}</div>
        </div>
      </div>

      {isPdf && <PdfPreview path={filePath} />}

      <div className="file-preview-meta">
        {result.modified && <span><span className="file-preview-meta-key">Modified </span>{formatDate(result.modified)}</span>}
        {result.created && <span><span className="file-preview-meta-key">Created </span>{formatDate(result.created)}</span>}
        {!isFolder && result.file_size != null && <span><span className="file-preview-meta-key">Size </span>{formatBytes(result.file_size)}</span>}
        <span><span className="file-preview-meta-key">Kind </span>{kind}</span>
      </div>
    </div>
  );
}

function PreviewPanel({ result, onLaunch }: { result: SearchResult | null; onLaunch: () => void }) {
  if (result?.kind === "app") {
    return <AppPreview result={result} onLaunch={onLaunch} />;
  }
  if (result?.kind === "file" || result?.kind === "folder") {
    return <FilePreview result={result} />;
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
            {results.map((result, i) => {
              const label = groupLabel(result.kind);
              const prevLabel = i > 0 ? groupLabel(results[i - 1].kind) : null;
              const showLabel = label !== null && label !== prevLabel;
              return (
                <Fragment key={result.id}>
                  {showLabel && (
                    <div className={`result-group-label${i === 0 ? " first" : ""}`}>
                      <span>{label}</span>
                    </div>
                  )}
                  <div
                    className={`result-row${i === selectedIndex ? " selected" : ""}`}
                    role="option"
                    aria-selected={i === selectedIndex}
                    onClick={() => { setSelectedIndex(i); launch(result.exec); }}
                  >
                    <ResultIcon icon_path={result.icon_path} title={result.title} kind={result.kind} />
                    <div className="result-text">
                      <div className="result-title">{result.title}</div>
                      {result.subtitle && (
                        <div className="result-subtitle">{shortenPath(result.subtitle)}</div>
                      )}
                    </div>
                    <div className="result-meta">
                      {result.kind === "file" && result.file_size != null
                        ? formatBytes(result.file_size)
                        : ""}
                    </div>
                  </div>
                </Fragment>
              );
            })}
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
