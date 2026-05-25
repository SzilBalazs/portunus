import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SearchResult } from "../types";
import { formatBytes, formatDate, fileKind, textPreviewLang, isImagePreviewable } from "../utils";
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
hljs.registerLanguage("plaintext",  langPlain);

// ── pdf ───────────────────────────────────────────────────────────────────────

const pdfPromiseCache = new Map<string, Promise<string>>();
const pdfUrlCache = new Map<string, string>();

function getPdfUrl(path: string): Promise<string> {
  if (!pdfPromiseCache.has(path)) {
    pdfPromiseCache.set(
      path,
      invoke<number[]>("render_pdf_page", { path })
        .then((bytes) => {
          const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: "image/jpeg" }));
          pdfUrlCache.set(path, url);
          return url;
        })
        .catch((e) => {
          pdfPromiseCache.delete(path);
          throw e;
        }),
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

  const isLoading = !src && !error;

  return (
    <div className={`pdf-preview-wrap${isLoading ? " is-loading" : ""}`}>
      {isLoading && <div className="pdf-skeleton" />}
      {src && (
        <img
          src={src}
          alt="Image preview"
          style={{ opacity: loaded ? 1 : 0 }}
          onLoad={() => setLoaded(true)}
        />
      )}
      {error && <span className="pdf-preview-msg">Preview unavailable</span>}
    </div>
  );
}

// ── text preview ─────────────────────────────────────────────────────────────

function TextPreview({ path, lang }: { path: string; lang: string }) {
  const [html, setHtml] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setHtml(null);
    invoke<string>("read_text_preview", { path })
      .then(text => {
        if (cancelled) return;
        const result = hljs.highlight(text, { language: lang, ignoreIllegals: true });
        setHtml(result.value);
      })
      .catch(() => { if (!cancelled) setHtml(""); });
    return () => { cancelled = true; };
  }, [path, lang]);

  if (html === null) return <div className="text-preview-wrap" />;

  return (
    <div className="text-preview-wrap">
      <pre className="text-preview-code hljs" dangerouslySetInnerHTML={{ __html: html }} />
    </div>
  );
}

// ── file / folder ─────────────────────────────────────────────────────────────

interface Props {
  result: SearchResult;
}

export default function FilePreview({ result }: Props) {
  const isFolder = result.kind === "folder";
  const kind = fileKind(result.title, isFolder);
  const tag = [kind, !isFolder && result.file_size != null ? formatBytes(result.file_size) : null]
    .filter(Boolean)
    .join(" · ");
  const filePath = result.subtitle ? `${result.subtitle}/${result.title}` : result.title;
  const isPdf = kind === "PDF Document";
  const isImage = !isFolder && isImagePreviewable(result.title);
  const textLang = !isFolder && !isImage ? textPreviewLang(result.title) : null;

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
      {isImage && <ImagePreview path={filePath} />}
      {textLang && <TextPreview path={filePath} lang={textLang} />}

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
