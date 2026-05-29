import { useState, useEffect, useLayoutEffect, useRef } from "react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { Components } from "react-markdown";
import { SearchResult } from "../types";
import { formatBytes, formatDate, fileKind, textPreviewLang, isImagePreviewable, isSvg, isCsv, isOfficeText, isSpreadsheet } from "../utils";
import { highlightInElement, cellMatches, buildTermRegex } from "../highlight";

/**
 * After the referenced element renders, wraps matched terms in `<mark>` and
 * scrolls the first into view. The caller must remount the highlighted subtree
 * (via a `key`) whenever content or terms change, so the effect always runs on
 * clean, React-untouched DOM.
 */
function useTermHighlight<T extends HTMLElement>(terms: string[], dep: unknown) {
  const ref = useRef<T>(null);
  // Layout effect so the marks + scroll land before the browser paints — using a
  // plain effect leaves one frame of un-highlighted, top-scrolled content (a jump).
  useLayoutEffect(() => {
    if (!ref.current || !terms.length) return;
    const first = highlightInElement(ref.current, terms);
    first?.scrollIntoView({ block: "center" });
  }, [dep, terms]);
  return ref;
}

/** Splits text into nodes with matched terms wrapped in `<mark class="preview-hl">`. */
function highlightText(text: string, terms: string[]): ReactNode {
  const re = buildTermRegex(terms);
  if (!re) return text;
  const out: ReactNode[] = [];
  let last = 0;
  let key = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) out.push(text.slice(last, m.index));
    out.push(<mark key={key++} className="preview-hl">{m[0]}</mark>);
    last = m.index + m[0].length;
    if (m[0].length === 0) re.lastIndex++;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}
import { EnterIcon, CopyIcon, FolderOpenIcon, CheckIcon } from "../icons";
import hljs from "highlight.js/lib/core";
import langRust       from "highlight.js/lib/languages/rust";
import langTS         from "highlight.js/lib/languages/typescript";
import langJS         from "highlight.js/lib/languages/javascript";
import langPy         from "highlight.js/lib/languages/python";
import langGo         from "highlight.js/lib/languages/go";
import langBash       from "highlight.js/lib/languages/bash";
import langJson       from "highlight.js/lib/languages/json";
import langIni        from "highlight.js/lib/languages/ini";
import langYaml       from "highlight.js/lib/languages/yaml";
import langMd         from "highlight.js/lib/languages/markdown";
import langCss        from "highlight.js/lib/languages/css";
import langXml        from "highlight.js/lib/languages/xml";
import langC          from "highlight.js/lib/languages/c";
import langCpp        from "highlight.js/lib/languages/cpp";
import langSql        from "highlight.js/lib/languages/sql";
import langPhp        from "highlight.js/lib/languages/php";
import langLua        from "highlight.js/lib/languages/lua";
import langSwift      from "highlight.js/lib/languages/swift";
import langRuby       from "highlight.js/lib/languages/ruby";
import langJava       from "highlight.js/lib/languages/java";
import langKotlin     from "highlight.js/lib/languages/kotlin";
import langDocker     from "highlight.js/lib/languages/dockerfile";
import langMake       from "highlight.js/lib/languages/makefile";
import langScss       from "highlight.js/lib/languages/scss";
import langLess       from "highlight.js/lib/languages/less";
import langPlain      from "highlight.js/lib/languages/plaintext";

hljs.registerLanguage("rust",       langRust);
hljs.registerLanguage("typescript", langTS);
hljs.registerLanguage("javascript", langJS);
hljs.registerLanguage("python",     langPy);
hljs.registerLanguage("go",         langGo);
hljs.registerLanguage("bash",       langBash);
hljs.registerLanguage("json",       langJson);
hljs.registerLanguage("ini",        langIni);
hljs.registerLanguage("yaml",       langYaml);
hljs.registerLanguage("markdown",   langMd);
hljs.registerLanguage("css",        langCss);
hljs.registerLanguage("xml",        langXml);
hljs.registerLanguage("c",          langC);
hljs.registerLanguage("cpp",        langCpp);
hljs.registerLanguage("sql",        langSql);
hljs.registerLanguage("php",        langPhp);
hljs.registerLanguage("lua",        langLua);
hljs.registerLanguage("swift",      langSwift);
hljs.registerLanguage("ruby",       langRuby);
hljs.registerLanguage("java",       langJava);
hljs.registerLanguage("kotlin",     langKotlin);
hljs.registerLanguage("dockerfile", langDocker);
hljs.registerLanguage("makefile",   langMake);
hljs.registerLanguage("scss",       langScss);
hljs.registerLanguage("less",       langLess);
hljs.registerLanguage("plaintext",  langPlain);

// ── pdf ───────────────────────────────────────────────────────────────────────

// Cache keyed by `path#page` so the same PDF previewed on different pages doesn't
// collide. Page count is cached per path (independent of which page is rendered).
const pdfPromiseCache = new Map<string, Promise<string>>();
const pdfUrlCache = new Map<string, string>();
const pdfPageCount = new Map<string, number>();

function pdfKey(path: string, page: number): string {
  return `${path}#${page}`;
}

function getPdfUrl(path: string, page: number): Promise<string> {
  const key = pdfKey(path, page);
  if (!pdfPromiseCache.has(key)) {
    pdfPromiseCache.set(
      key,
      invoke<[number[], number]>("render_pdf_page", { path, page })
        .then(([bytes, count]) => {
          pdfPageCount.set(path, count);
          const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: "image/jpeg" }));
          pdfUrlCache.set(key, url);
          return url;
        })
        .catch((e) => {
          pdfPromiseCache.delete(key);
          throw e;
        }),
    );
  }
  return pdfPromiseCache.get(key)!;
}

function PdfPreview({ path, page }: { path: string; page: number }) {
  // `cur` is the displayed page; starts at the matched page and moves with Ctrl+←/→.
  const [cur, setCur] = useState(page);
  const [count, setCount] = useState<number | null>(() => pdfPageCount.get(path) ?? null);
  const key = pdfKey(path, cur);
  const [src, setSrc] = useState<string | null>(() => pdfUrlCache.get(key) ?? null);
  const [loaded, setLoaded] = useState(() => pdfUrlCache.has(key));
  const [error, setError] = useState(false);

  // Reset to the matched page when the previewed file changes.
  useEffect(() => { setCur(page); }, [path, page]);

  useEffect(() => {
    const cached = pdfUrlCache.get(key);
    if (cached) {
      setSrc(cached);
      setLoaded(true);
      setError(false);
      setCount(pdfPageCount.get(path) ?? null);
      return;
    }
    let cancelled = false;
    setSrc(null);
    setLoaded(false);
    setError(false);
    getPdfUrl(path, cur)
      .then((url) => { if (!cancelled) { setSrc(url); setCount(pdfPageCount.get(path) ?? null); } })
      .catch((e) => {
        console.error("[pdf] render_pdf_page failed:", e);
        if (!cancelled) setError(true);
      });
    return () => { cancelled = true; };
  }, [key, path, cur]);

  // Ctrl+←/→ flips pages, clamped to [0, count-1] once the count is known.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
      const max = count != null ? count - 1 : Infinity;
      setCur((c) => {
        const next = e.key === "ArrowRight" ? Math.min(c + 1, max) : Math.max(c - 1, 0);
        return next;
      });
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [count]);

  return (
    <div className={`pdf-preview-wrap${!loaded && !error ? " is-loading" : ""}`}>
      {!error && (
        <div
          className="pdf-skeleton"
          style={{ opacity: loaded ? 0 : 1, animation: loaded ? "none" : undefined }}
        />
      )}
      {src && (
        <img
          src={src}
          alt="PDF preview"
          className={loaded ? "pdf-img-revealed" : undefined}
          style={{ opacity: loaded ? undefined : 0 }}
          onLoad={() => setLoaded(true)}
        />
      )}
      {loaded && (
        <span className="pdf-page-label">
          {count != null ? `Page ${cur + 1} / ${count}` : `Page ${cur + 1}`}
        </span>
      )}
      {error && <span className="pdf-preview-msg">Preview unavailable</span>}
    </div>
  );
}

// ── image preview ────────────────────────────────────────────────────────────

const imgPromiseCache = new Map<string, Promise<string>>();
const imgUrlCache = new Map<string, string>();

function getImgUrl(path: string): Promise<string> {
  if (!imgPromiseCache.has(path)) {
    imgPromiseCache.set(
      path,
      invoke<number[]>("render_image_preview", { path })
        .then(bytes => {
          const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: "image/jpeg" }));
          imgUrlCache.set(path, url);
          return url;
        })
        .catch(e => {
          imgPromiseCache.delete(path);
          throw e;
        }),
    );
  }
  return imgPromiseCache.get(path)!;
}

function ImagePreview({ path }: { path: string }) {
  const [src, setSrc] = useState<string | null>(() => imgUrlCache.get(path) ?? null);
  const [loaded, setLoaded] = useState(() => imgUrlCache.has(path));
  const [error, setError] = useState(false);

  useEffect(() => {
    const cached = imgUrlCache.get(path);
    if (cached) { setSrc(cached); setLoaded(true); setError(false); return; }
    let cancelled = false;
    setSrc(null); setLoaded(false); setError(false);
    getImgUrl(path)
      .then(url => { if (!cancelled) setSrc(url); })
      .catch(() => { if (!cancelled) setError(true); });
    return () => { cancelled = true; };
  }, [path]);

  return (
    <div className={`pdf-preview-wrap${!loaded && !error ? " is-loading" : ""}`}>
      {!error && (
        <div
          className="pdf-skeleton"
          style={{ opacity: loaded ? 0 : 1, animation: loaded ? "none" : undefined }}
        />
      )}
      {src && (
        <img
          src={src}
          alt="Image preview"
          className={loaded ? "pdf-img-revealed" : undefined}
          style={{ opacity: loaded ? undefined : 0 }}
          onLoad={() => setLoaded(true)}
        />
      )}
      {error && <span className="pdf-preview-msg">Preview unavailable</span>}
    </div>
  );
}

// ── svg preview ───────────────────────────────────────────────────────────────

// The asset protocol scope is restricted to icon dirs, so we can't use
// convertFileSrc for arbitrary paths. Instead, read the SVG markup via the
// existing read_text_preview command and create a blob URL from it.
const svgBlobCache = new Map<string, string>();

function SvgPreview({ path }: { path: string }) {
  const [src, setSrc] = useState<string | null>(() => svgBlobCache.get(path) ?? null);
  const [loaded, setLoaded] = useState(() => svgBlobCache.has(path));
  const [error, setError] = useState(false);

  useEffect(() => {
    const cached = svgBlobCache.get(path);
    if (cached) { setSrc(cached); setLoaded(true); setError(false); return; }
    let cancelled = false;
    setSrc(null); setLoaded(false); setError(false);
    invoke<string>("read_text_preview", { path })
      .then(markup => {
        if (cancelled) return;
        const blob = new Blob([markup], { type: "image/svg+xml" });
        const url = URL.createObjectURL(blob);
        svgBlobCache.set(path, url);
        setSrc(url);
      })
      .catch(() => { if (!cancelled) setError(true); });
    return () => { cancelled = true; };
  }, [path]);

  return (
    <div className={`pdf-preview-wrap${!loaded && !error ? " is-loading" : ""}`}>
      {!error && (
        <div
          className="pdf-skeleton"
          style={{ opacity: loaded ? 0 : 1, animation: loaded ? "none" : undefined }}
        />
      )}
      {src && (
        <img
          src={src}
          alt="SVG preview"
          className={loaded ? "pdf-img-revealed" : undefined}
          style={{ opacity: loaded ? undefined : 0 }}
          onLoad={() => setLoaded(true)}
          onError={() => setError(true)}
        />
      )}
      {error && <span className="pdf-preview-msg">Preview unavailable</span>}
    </div>
  );
}

// ── csv / tsv preview ─────────────────────────────────────────────────────────

// Minimal RFC-4180-ish parser: handles quoted fields with embedded delimiters
// and doubled "" escapes. Good enough for a preview over the (possibly line-
// truncated) text returned by read_text_preview.
function parseDelimited(text: string, delim: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let inQuotes = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (inQuotes) {
      if (ch === '"') {
        if (text[i + 1] === '"') { field += '"'; i++; }
        else inQuotes = false;
      } else field += ch;
    } else if (ch === '"') {
      inQuotes = true;
    } else if (ch === delim) {
      row.push(field); field = "";
    } else if (ch === "\n") {
      row.push(field); field = "";
      rows.push(row); row = [];
    } else if (ch !== "\r") {
      field += ch;
    }
  }
  if (field.length > 0 || row.length > 0) { row.push(field); rows.push(row); }
  return rows;
}

// Shared table renderer for CSV/TSV and spreadsheet previews. Highlights matched
// terms per cell and scrolls the first matching cell into view.
function DataTable({ rows, terms }: { rows: string[][]; terms: string[] }) {
  const firstMatch = useRef<HTMLTableCellElement | null>(null);

  useEffect(() => {
    firstMatch.current?.scrollIntoView({ block: "center" });
  }, [rows, terms]);

  let found = false;
  // Returns a ref callback for the first term-matching cell, undefined otherwise.
  const refForCell = (text: string) => {
    if (found || !terms.length || !cellMatches(text, terms)) return undefined;
    found = true;
    return (el: HTMLTableCellElement | null) => { firstMatch.current = el; };
  };

  const cellClass = (text: string) =>
    terms.length && cellMatches(text, terms) ? "preview-hl-cell" : undefined;

  const [header, ...body] = rows;
  return (
    <div className="text-preview-wrap">
      <table className="csv-preview">
        <thead>
          <tr>
            {header.map((cell, i) => (
              <th key={i} ref={refForCell(cell)} className={cellClass(cell)}>{highlightText(cell, terms)}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {body.map((r, ri) => (
            <tr key={ri}>
              {r.map((cell, ci) => (
                <td key={ci} ref={refForCell(cell)} className={cellClass(cell)}>{highlightText(cell, terms)}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function CsvPreview({ path, delim, terms }: { path: string; delim: string; terms: string[] }) {
  const [rows, setRows] = useState<string[][] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setRows(null);
    invoke<string>("read_text_preview", { path })
      .then(text => { if (!cancelled) setRows(parseDelimited(text, delim).slice(0, 100)); })
      .catch(() => { if (!cancelled) setRows([]); });
    return () => { cancelled = true; };
  }, [path, delim]);

  if (rows === null || rows.length === 0) return <div className="text-preview-wrap" />;
  return <DataTable rows={rows} terms={terms} />;
}

// ── office text preview (docx / pptx / odt / odp) ────────────────────────────

function OfficeTextPreview({ path, terms }: { path: string; terms: string[] }) {
  const [source, setSource] = useState<string | null>(null);
  const ref = useTermHighlight<HTMLDivElement>(terms, source);

  useEffect(() => {
    let cancelled = false;
    setSource(null);
    // Backend returns the document as Markdown; render it like a .md file.
    invoke<string>("read_office_preview", { path })
      .then(text => { if (!cancelled) setSource(text); })
      .catch(() => { if (!cancelled) setSource(""); });
    return () => { cancelled = true; };
  }, [path]);

  if (source === null) return <div className="text-preview-wrap" />;

  const baseDir = path.slice(0, path.lastIndexOf("/")) || "/";
  const mdComponents: Components = {
    code: mdCodeComponent,
    img: ({ src, alt }) => <MarkdownImage src={typeof src === "string" ? src : undefined} alt={alt} baseDir={baseDir} />,
  };

  return (
    <div className="text-preview-wrap">
      <div className="md-preview-wrap" ref={ref} key={`${source}${terms.join("")}`}>
        <ReactMarkdown remarkPlugins={[remarkGfm]} components={mdComponents}>{source}</ReactMarkdown>
      </div>
    </div>
  );
}

// ── spreadsheet preview (xlsx / ods) ─────────────────────────────────────────

function SpreadsheetPreview({ path, terms }: { path: string; terms: string[] }) {
  const [rows, setRows] = useState<string[][] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setRows(null);
    invoke<string[][]>("read_spreadsheet_preview", { path })
      .then(r => { if (!cancelled) setRows(r); })
      .catch(() => { if (!cancelled) setRows([]); });
    return () => { cancelled = true; };
  }, [path]);

  if (rows === null || rows.length === 0) return <div className="text-preview-wrap" />;
  return <DataTable rows={rows} terms={terms} />;
}

// ── folder contents ───────────────────────────────────────────────────────────

interface FolderEntry { name: string; is_dir: boolean; size?: number; }

function FolderContents({ path }: { path: string }) {
  const [entries, setEntries] = useState<FolderEntry[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setEntries(null);
    invoke<FolderEntry[]>("list_folder", { path })
      .then(e => { if (!cancelled) setEntries(e); })
      .catch(() => { if (!cancelled) setEntries([]); });
    return () => { cancelled = true; };
  }, [path]);

  return (
    <div className="folder-contents">
      {entries === null && <div className="folder-contents-empty">Loading…</div>}
      {entries !== null && entries.length === 0 && (
        <div className="folder-contents-empty">Empty folder</div>
      )}
      {entries?.map(e => (
        <div key={e.name} className="folder-entry">
          <span className="folder-entry-icon">
            {e.is_dir ? (
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="13" height="13">
                <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
              </svg>
            ) : (
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" width="13" height="13">
                <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                <polyline points="14 2 14 8 20 8" />
              </svg>
            )}
          </span>
          <span className="folder-entry-name">{e.name}</span>
          {!e.is_dir && e.size != null && (
            <span className="folder-entry-size">{formatBytes(e.size)}</span>
          )}
        </div>
      ))}
    </div>
  );
}

// ── markdown preview ──────────────────────────────────────────────────────────

const mdCodeComponent: Components["code"] = ({ className, children }) => {
  const match = /language-(\w+)/.exec(className ?? "");
  if (match) {
    try {
      const highlighted = hljs.highlight(String(children).replace(/\n$/, ""), {
        language: match[1],
        ignoreIllegals: true,
      });
      return (
        <code
          className={`hljs language-${match[1]}`}
          dangerouslySetInnerHTML={{ __html: highlighted.value }}
        />
      );
    } catch { /* fall through to plain */ }
  }
  return <code className={className}>{children}</code>;
};

// Resolve a markdown image src against the document's directory and load it via
// render_image_preview (asset protocol is scoped to icon dirs, so local paths
// can't be used directly). Remote (http/data) srcs pass through unchanged.
function MarkdownImage({ src, alt, baseDir }: { src?: string; alt?: string; baseDir: string }) {
  const isRemote = !!src && /^(https?:|data:)/.test(src);
  const [resolved, setResolved] = useState<string | null>(isRemote ? src! : null);

  useEffect(() => {
    if (!src || isRemote) { setResolved(src ?? null); return; }
    let cancelled = false;
    setResolved(null);
    const abs = src.startsWith("/") ? src : `${baseDir}/${src}`;
    invoke<number[]>("render_image_preview", { path: abs })
      .then(bytes => {
        if (cancelled) return;
        const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: "image/jpeg" }));
        setResolved(url);
      })
      .catch(() => { if (!cancelled) setResolved(null); });
    return () => { cancelled = true; };
  }, [src, baseDir, isRemote]);

  if (!resolved) return null;
  return <img src={resolved} alt={alt ?? ""} />;
}

function MarkdownPreview({ path, terms }: { path: string; terms: string[] }) {
  const [source, setSource] = useState<string | null>(null);
  const ref = useTermHighlight<HTMLDivElement>(terms, source);
  const baseDir = path.slice(0, path.lastIndexOf("/")) || "/";

  useEffect(() => {
    let cancelled = false;
    setSource(null);
    invoke<string>("read_text_preview", { path, terms })
      .then(text => { if (!cancelled) setSource(text); })
      .catch(() => { if (!cancelled) setSource(""); });
    return () => { cancelled = true; };
  }, [path, terms]);

  if (source === null) return <div className="text-preview-wrap" />;

  const mdComponents: Components = {
    code: mdCodeComponent,
    img: ({ src, alt }) => <MarkdownImage src={typeof src === "string" ? src : undefined} alt={alt} baseDir={baseDir} />,
  };

  return (
    <div className="text-preview-wrap">
      <div className="md-preview-wrap" ref={ref} key={`${source}${terms.join("")}`}>
        <ReactMarkdown remarkPlugins={[remarkGfm]} components={mdComponents}>{source}</ReactMarkdown>
      </div>
    </div>
  );
}

// ── text preview ─────────────────────────────────────────────────────────────

function TextPreview({ path, lang, terms }: { path: string; lang: string; terms: string[] }) {
  const [html, setHtml] = useState<string | null>(null);
  const ref = useTermHighlight<HTMLPreElement>(terms, html);

  useEffect(() => {
    let cancelled = false;
    setHtml(null);
    // Pass terms so the backend returns a window centered on the first match.
    invoke<string>("read_text_preview", { path, terms })
      .then(text => {
        if (cancelled) return;
        setTimeout(() => {
          if (cancelled) return;
          setHtml(hljs.highlight(text, { language: lang, ignoreIllegals: true }).value);
        }, 0);
      })
      .catch(() => { if (!cancelled) setHtml(""); });
    return () => { cancelled = true; };
  }, [path, lang, terms]);

  if (html === null) return <div className="text-preview-wrap" />;

  return (
    <div className="text-preview-wrap">
      <pre
        ref={ref}
        className="text-preview-code hljs"
        dangerouslySetInnerHTML={{ __html: html }}
        key={`${html.length}${terms.join("")}`}
      />
    </div>
  );
}

// ── file / folder ─────────────────────────────────────────────────────────────

interface Props {
  result: SearchResult;
  onLaunch: () => void;
  onReveal?: () => void;
  /** Matched content-search terms to highlight (empty for non-content searches). */
  terms?: string[];
}

export default function FilePreview({ result, onLaunch, onReveal, terms = [] }: Props) {
  const isFolder = result.kind === "folder";
  const kind = fileKind(result.title, isFolder);
  const tag = [kind, !isFolder && result.file_size != null ? formatBytes(result.file_size) : null]
    .filter(Boolean)
    .join(" · ");
  const filePath = result.subtitle ? `${result.subtitle}/${result.title}` : result.title;
  const isPdf = kind === "PDF Document";
  const isImage = !isFolder && isImagePreviewable(result.title);
  const isSvgFile = !isFolder && isSvg(result.title);
  const isCsvFile = !isFolder && isCsv(result.title);
  const isOfficeTextFile = !isFolder && isOfficeText(result.title);
  const isSpreadsheetFile = !isFolder && isSpreadsheet(result.title);
  const textLang = !isFolder && !isImage && !isSvgFile && !isCsvFile && !isOfficeTextFile && !isSpreadsheetFile
    ? textPreviewLang(result.title)
    : null;

  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    navigator.clipboard.writeText(filePath);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const handleReveal = () => {
    const parent = result.subtitle ?? '.';
    invoke('launch_app', { exec: `xdg-open "${parent}"`, id: undefined, kind: undefined });
    onReveal?.();
  };

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
          {result.snippet && <div className="file-preview-path">{filePath}</div>}
        </div>
        <div className="file-preview-actions">
          <button className={`file-btn-icon${copied ? ' copied' : ''}`} onClick={handleCopy} title="Copy path" tabIndex={-1}>
            {copied ? <CheckIcon /> : <CopyIcon />}
          </button>
          {!isFolder && (
            <button className="file-btn-icon" onClick={handleReveal} title="Reveal in folder" tabIndex={-1}>
              <FolderOpenIcon />
            </button>
          )}
          <button className="btn-primary" onClick={onLaunch} tabIndex={-1}>
            Open <span className="btn-kbd"><EnterIcon /></span>
          </button>
        </div>
      </div>

      {isPdf && <PdfPreview path={filePath} page={result.match_page ?? 0} />}
      {isImage && <ImagePreview path={filePath} />}
      {isSvgFile && <SvgPreview path={filePath} />}
      {isCsvFile && <CsvPreview path={filePath} delim={result.title.toLowerCase().endsWith(".tsv") ? "\t" : ","} terms={terms} />}
      {isOfficeTextFile && <OfficeTextPreview path={filePath} terms={terms} />}
      {isSpreadsheetFile && <SpreadsheetPreview path={filePath} terms={terms} />}
      {textLang === "markdown" && <MarkdownPreview path={filePath} terms={terms} />}
      {textLang && textLang !== "markdown" && <TextPreview path={filePath} lang={textLang} terms={terms} />}
      {isFolder && <FolderContents path={filePath} />}

      <div className="file-preview-meta">
        {result.modified && (
          <span><span className="file-preview-meta-key">Modified </span>{formatDate(result.modified)}</span>
        )}
        {result.created && (
          <span><span className="file-preview-meta-key">Created </span>{formatDate(result.created)}</span>
        )}
        {!isFolder && result.file_size != null && (
          <span><span className="file-preview-meta-key">Size </span>{formatBytes(result.file_size)}</span>
        )}
        <span><span className="file-preview-meta-key">Kind </span>{kind}</span>
      </div>
    </div>
  );
}
