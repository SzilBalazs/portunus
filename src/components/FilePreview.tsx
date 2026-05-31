import { useState, useEffect, useLayoutEffect, useRef } from "react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { Components } from "react-markdown";
import { SearchResult } from "../types";
import { formatBytes, formatDate, fileKind, textPreviewLang, isImagePreviewable, isSvg, isCsv, isOfficeText, isSpreadsheet } from "../utils";
import { highlightInElement, focusBestCluster, cellMatches, buildTermRegex } from "../highlight";

/**
 * After the referenced element renders, wraps matched terms in `<mark>` and
 * scrolls the densest section (most distinct terms) into view. The caller must
 * remount the highlighted subtree (via a `key`) whenever content or terms change,
 * so the effect always runs on clean, React-untouched DOM.
 */
function useTermHighlight<T extends HTMLElement>(terms: string[], dep: unknown) {
  const ref = useRef<T>(null);
  // Layout effect so the marks + scroll land before the browser paints — using a
  // plain effect leaves one frame of un-highlighted, top-scrolled content (a jump).
  useLayoutEffect(() => {
    if (!ref.current || !terms.length) return;
    highlightInElement(ref.current, terms);
    focusBestCluster(ref.current, terms)?.scrollIntoView({ block: "center" });
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

// Cache keyed by `path#page@width` so the same page rendered at different widths
// (small side preview vs high-DPI zoomed Quicklook) doesn't collide. Page count
// is cached per path (independent of which page/width is rendered).
const pdfPromiseCache = new Map<string, Promise<string>>();
const pdfUrlCache = new Map<string, string>();
const pdfPageCount = new Map<string, number>();

function pdfKey(path: string, page: number, width: number): string {
  return `${path}#${page}@${width}`;
}

// Currently-previewed PDF page, read by App.launch to open at the right page.
export const pdfView = { path: "", page: 0 };

// Count of mounted Quicklook PDF previews. The side-panel preview stays mounted
// under the overlay; without this both would react to Ctrl+←/→ and fight over the
// page. The side preview defers its page-nav while a Quicklook is open.
let pdfQuicklookMounted = 0;

// Fired when a Quicklook PDF unmounts so the side preview can re-sync its page to
// pdfView (which holds the page the user navigated to in Quicklook).
const PDF_SYNC_EVENT = "portunus-pdf-sync";

const PDF_QL_FIXED_ZOOM = 0.75;

// Cap the rendered-page cache; evict the oldest blob URLs (and revoke them) so a
// long session of zooming/paging across PDFs doesn't leak detached bitmaps.
const PDF_URL_CACHE_CAP = 64;
function storePdfUrl(key: string, url: string) {
  pdfUrlCache.set(key, url);
  while (pdfUrlCache.size > PDF_URL_CACHE_CAP) {
    const oldest = pdfUrlCache.keys().next().value as string | undefined;
    if (oldest === undefined) break;
    const u = pdfUrlCache.get(oldest);
    if (u) URL.revokeObjectURL(u);
    pdfUrlCache.delete(oldest);
    pdfPromiseCache.delete(oldest);
  }
}

function getPdfUrl(path: string, page: number, width: number): Promise<string> {
  const key = pdfKey(path, page, width);
  if (!pdfPromiseCache.has(key)) {
    pdfPromiseCache.set(
      key,
      invoke<[number[], number]>("render_pdf_page", { path, page, width })
        .then(([bytes, count]) => {
          pdfPageCount.set(path, count);
          const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: "image/jpeg" }));
          storePdfUrl(key, url);
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

function PdfPreview({ path, page, quicklook = false }: { path: string; page: number; quicklook?: boolean }) {
  // `cur` is the displayed page; moves with Ctrl+←/→. Seed from the live page the
  // side preview last showed for this file (via pdfView) so opening Quicklook keeps
  // the current page — falling back to the content-match page for a fresh file.
  const startPage = () => (pdfView.path === path ? pdfView.page : page);
  const [cur, setCur] = useState(startPage);
  const [count, setCount] = useState<number | null>(() => pdfPageCount.get(path) ?? null);
  const [aspect, setAspect] = useState(0);

  // Quicklook measures its reader surface to size renders; side preview uses the
  // fixed 800px default (downscaled to fit, so resolution is plenty).
  // outerRef is the non-scrolling parent: measuring IT (not the scroll container)
  // means the scrollbar appearing/disappearing can't change the measured width —
  // otherwise width↔scrollbar feedback makes the page jump every frame.
  const outerRef = useRef<HTMLDivElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const imgRef = useRef<HTMLImageElement>(null);
  const [vp, setVp] = useState({ w: 0, h: 0 });
  // Fixed gutter reserved for the scrollbar, so a vertical scrollbar never pushes
  // the page into horizontal overflow.
  const SCROLLBAR_RESERVE = 14;
  useLayoutEffect(() => {
    if (!quicklook) return;
    const outer = outerRef.current;
    const inner = wrapRef.current;
    if (!outer || !inner) return;
    const measure = () => {
      const cs = getComputedStyle(inner);
      const px = parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight);
      const py = parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom);
      const w = Math.max(0, outer.clientWidth - px - SCROLLBAR_RESERVE);
      const h = Math.max(0, outer.clientHeight - py);
      // Skip no-op updates so a stable size can't churn renders / re-measures.
      setVp(prev => (prev.w === w && prev.h === h ? prev : { w, h }));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(outer);
    return () => ro.disconnect();
  }, [quicklook]);

  const displayW = quicklook && vp.w > 0 ? Math.round(vp.w * PDF_QL_FIXED_ZOOM) : 0;
  const displayH = aspect > 0 && displayW > 0 ? Math.round(displayW / aspect) : undefined;
  const renderWidth = quicklook && displayW > 0 ? Math.max(1200, displayW) : 800;

  const key = pdfKey(path, cur, renderWidth);
  const [src, setSrc] = useState<string | null>(() => pdfUrlCache.get(key) ?? null);
  const srcRef = useRef(src);
  useEffect(() => { srcRef.current = src; }, [src]);
  const [loaded, setLoaded] = useState(() => pdfUrlCache.has(key));
  const [rendering, setRendering] = useState(false);
  // Side-preview-only: delayed skeleton + reveal animation.
  const [showSkeleton, setShowSkeleton] = useState(false);
  const [reveal, setReveal] = useState(false);
  const skeletonShownRef = useRef(false);
  const [error, setError] = useState(false);

  useEffect(() => { setCur(startPage()); }, [path, page]);

  // Track the displayed page so launch can open the PDF at it.
  useEffect(() => { pdfView.path = path; pdfView.page = cur; }, [path, cur]);

  // Register this mount for page-nav arbitration (see pdfQuicklookMounted), and on
  // unmount tell the side preview to adopt the page we ended on.
  useEffect(() => {
    if (!quicklook) return;
    pdfQuicklookMounted++;
    return () => { pdfQuicklookMounted--; window.dispatchEvent(new Event(PDF_SYNC_EVENT)); };
  }, [quicklook]);

  // Side preview: when a Quicklook closes, sync to the page it left off on.
  useEffect(() => {
    if (quicklook) return;
    const onSync = () => { if (pdfView.path === path) setCur(pdfView.page); };
    window.addEventListener(PDF_SYNC_EVENT, onSync);
    return () => window.removeEventListener(PDF_SYNC_EVENT, onSync);
  }, [quicklook, path]);

  useEffect(() => {
    // Quicklook waits for its measurement so the first render targets the real width.
    if (quicklook && vp.w === 0) return;
    const cached = pdfUrlCache.get(key);
    if (cached) {
      setSrc(cached); setLoaded(true); setRendering(false);
      setShowSkeleton(false); setReveal(false); setError(false);
      setCount(pdfPageCount.get(path) ?? null);
      return;
    }
    let cancelled = false;
    setError(false);

    if (quicklook) {
      // Zoom/page re-render: keep the current page visible (CSS-scaled) so zooming
      // feels instant and never blanks; just swap to the sharp bitmap when ready.
      setRendering(true);
      const renderTimer = setTimeout(() => {
        getPdfUrl(path, cur, renderWidth)
          .then(async (url) => {
            try { const probe = new Image(); probe.src = url; await probe.decode(); } catch { /* onLoad covers it */ }
            if (cancelled) return;
            setSrc(url); setLoaded(true); setRendering(false);
            setCount(pdfPageCount.get(path) ?? null);
          })
          .catch((e) => {
            console.error("[pdf] render_pdf_page failed:", e);
            if (cancelled) return;
            setRendering(false);
            if (!srcRef.current) setError(true); // only when there's nothing to show
          });
      }, 80);
      return () => { cancelled = true; clearTimeout(renderTimer); };
    }

    // Side preview: keep the current image up; show a delayed skeleton only if the
    // render is slow enough to need masking.
    setReveal(false);
    skeletonShownRef.current = false;
    const skeletonTimer = setTimeout(() => {
      if (cancelled) return;
      skeletonShownRef.current = true;
      setSrc(null); setLoaded(false); setShowSkeleton(true);
    }, 140);
    const renderTimer = setTimeout(() => {
      getPdfUrl(path, cur, renderWidth)
        .then(async (url) => {
          try { const probe = new Image(); probe.src = url; await probe.decode(); } catch { /* onLoad covers it */ }
          if (cancelled) return;
          clearTimeout(skeletonTimer);
          setReveal(skeletonShownRef.current);
          setSrc(url); setLoaded(true); setShowSkeleton(false);
          setCount(pdfPageCount.get(path) ?? null);
        })
        .catch((e) => {
          console.error("[pdf] render_pdf_page failed:", e);
          if (!cancelled) { clearTimeout(skeletonTimer); setSrc(null); setLoaded(false); setShowSkeleton(false); setError(true); }
        });
    }, 80);
    return () => { cancelled = true; clearTimeout(skeletonTimer); clearTimeout(renderTimer); };
  }, [key, path, cur, renderWidth, quicklook, vp.w]);

  // Reset scroll to the top of the page when flipping pages, so the next page
  // starts at its top rather than wherever the previous one was scrolled.
  useEffect(() => {
    if (!quicklook) return;
    const el = wrapRef.current;
    if (el) el.scrollTop = 0;
  }, [cur, quicklook]);

  // Quicklook only: prefetch the adjacent pages at the current width so Ctrl+←/→
  // flips render instantly. Best-effort and cache-deduped; bounded to valid pages.
  useEffect(() => {
    if (!quicklook || vp.w === 0) return;
    const t = setTimeout(() => {
      for (const p of [cur - 1, cur + 1]) {
        if (p < 0 || (count != null && p > count - 1)) continue;
        if (!pdfUrlCache.has(pdfKey(path, p, renderWidth))) {
          getPdfUrl(path, p, renderWidth).catch(() => {});
        }
      }
    }, 150);
    return () => clearTimeout(t);
  }, [quicklook, path, cur, renderWidth, count, vp.w]);

  // Ctrl+←/→ flips pages, clamped to [0, count-1] once the count is known.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
      if (!document.hasFocus()) return;
      // While a Quicklook PDF is open, the background side preview ignores page-nav.
      if (!quicklook && pdfQuicklookMounted > 0) return;
      setCur((c) => {
        // Until the page count is known, don't advance past the current page —
        // otherwise rapid Ctrl+→ overshoots into a non-existent page and flashes
        // "Preview unavailable" until the count arrives.
        const max = count != null ? count - 1 : c;
        return e.key === "ArrowRight" ? Math.min(c + 1, max) : Math.max(c - 1, 0);
      });
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [count, quicklook]);

  if (quicklook) {
    return (
      <div className="pdf-ql-wrap" ref={outerRef}>
        <div className="pdf-ql" ref={wrapRef}>
          {src && (
            <img
              ref={imgRef}
              src={src}
              alt="PDF preview"
              className="pdf-ql-page"
              draggable={false}
              style={displayW ? { width: displayW, height: displayH } : undefined}
              onLoad={e => {
                setLoaded(true);
                const t = e.currentTarget;
                if (t.naturalWidth && t.naturalHeight) {
                  setAspect(t.naturalWidth / t.naturalHeight);
                }
              }}
            />
          )}
          {!src && !error && <div className="pdf-ql-spinner" />}
          {error && <span className="pdf-preview-msg">Preview unavailable</span>}
          {rendering && src && <div className="pdf-ql-rendering" />}
        </div>
        <div className="pdf-ql-hud">
          <span>{count != null ? `Page ${cur + 1} / ${count}` : `Page ${cur + 1}`}</span>
          <span className="pdf-ql-hud-keys"><kbd>ctrl</kbd><kbd>←→</kbd> page</span>
        </div>
      </div>
    );
  }

  return (
    <div className={`pdf-preview-wrap${showSkeleton && !loaded && !error ? " is-loading" : ""}`}>
      {!error && showSkeleton && (
        <div
          className="pdf-skeleton"
          style={{ opacity: loaded ? 0 : 1, animation: loaded ? "none" : undefined }}
        />
      )}
      {src && (
        <img
          src={src}
          alt="PDF preview"
          className={reveal ? "pdf-img-revealed" : undefined}
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

// Side preview renders at 800; Quicklook at a larger width so an enlarged image
// stays crisp instead of upscaling the 800px thumbnail.
const IMG_QL_WIDTH = 1600;

function imgKey(path: string, width: number): string { return `${path}@${width}`; }

function getImgUrl(path: string, width: number): Promise<string> {
  const key = imgKey(path, width);
  if (!imgPromiseCache.has(key)) {
    imgPromiseCache.set(
      key,
      invoke<number[]>("render_image_preview", { path, width })
        .then(bytes => {
          const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: "image/jpeg" }));
          imgUrlCache.set(key, url);
          return url;
        })
        .catch(e => {
          imgPromiseCache.delete(key);
          throw e;
        }),
    );
  }
  return imgPromiseCache.get(key)!;
}

function ImagePreview({ path, quicklook = false }: { path: string; quicklook?: boolean }) {
  const width = quicklook ? IMG_QL_WIDTH : 800;
  const key = imgKey(path, width);
  const [src, setSrc] = useState<string | null>(() => imgUrlCache.get(key) ?? null);
  const [loaded, setLoaded] = useState(() => imgUrlCache.has(key));
  const [error, setError] = useState(false);

  useEffect(() => {
    const cached = imgUrlCache.get(key);
    if (cached) { setSrc(cached); setLoaded(true); setError(false); return; }
    let cancelled = false;
    setSrc(null); setLoaded(false); setError(false);
    getImgUrl(path, width)
      .then(url => { if (!cancelled) setSrc(url); })
      .catch(() => { if (!cancelled) setError(true); });
    return () => { cancelled = true; };
  }, [key, path, width]);

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
  /** Rendered in the full-card Quicklook overlay — enables the large scrollable PDF reader. */
  quicklook?: boolean;
}

export default function FilePreview({ result, onLaunch, onReveal, terms = [], quicklook = false }: Props) {
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
    invoke('reveal_file', { path: filePath });
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
    <div className={`file-preview${quicklook ? " file-preview-ql" : ""}`}>
      {!quicklook && (
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
      )}

      {isPdf && <PdfPreview path={filePath} page={result.match_page ?? 0} quicklook={quicklook} />}
      {isImage && <ImagePreview path={filePath} quicklook={quicklook} />}
      {isSvgFile && <SvgPreview path={filePath} />}
      {isCsvFile && <CsvPreview path={filePath} delim={result.title.toLowerCase().endsWith(".tsv") ? "\t" : ","} terms={terms} />}
      {isOfficeTextFile && <OfficeTextPreview path={filePath} terms={terms} />}
      {isSpreadsheetFile && <SpreadsheetPreview path={filePath} terms={terms} />}
      {textLang === "markdown" && <MarkdownPreview path={filePath} terms={terms} />}
      {textLang && textLang !== "markdown" && <TextPreview path={filePath} lang={textLang} terms={terms} />}
      {isFolder && <FolderContents path={filePath} />}

      {!quicklook && (
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
      )}
    </div>
  );
}
