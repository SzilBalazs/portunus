import { useEffect, useLayoutEffect, useState, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { PreviewProps } from "../providers/registry";
import type { PreviewContent } from "../types";

// Per-id preview cache so flipping between results doesn't re-invoke the
// extension. Cleared wholesale when it grows - previews are tiny and queries
// are short-lived, an LRU would be overkill.
const cache = new Map<string, PreviewContent | null>();
const CACHE_MAX = 100;

// Extension reloads (Rescan button, `portunus --reload-extensions`) emit
// search-invalidated after swapping instances - drop cached previews and bump
// the version so even the currently-shown preview refetches (its result id is
// unchanged, so the id-keyed effect alone wouldn't rerun).
let cacheVersion = 0;
const versionListeners = new Set<() => void>();
void listen("search-invalidated", () => {
  cache.clear();
  cacheVersion++;
  versionListeners.forEach(l => l());
});
const subscribeVersion = (cb: () => void) => {
  versionListeners.add(cb);
  return () => void versionListeners.delete(cb);
};

/**
 * Renders the declarative preview an extension returned for the selected
 * result. Extensions never ship UI - they return data (markdown, metadata,
 * image, list) and this component renders it with the host's own widgets.
 * Raw HTML in markdown is NOT rendered (no rehype-raw) - extension content
 * stays inert.
 */

const THEME_VARS = [
  '--fg', '--fg-mute', '--fg-dim', '--fg-desc',
  '--bg', '--bg-deep', '--bg-card',
  '--accent', '--accent-soft', '--accent-border',
  '--radius', '--radius-sm', '--line', '--border', '--text-on-accent',
];

const EXT_UTILS_CSS = [
  '.text-mute{color:var(--fg-mute)}.text-dim{color:var(--fg-dim)}',
  '.text-desc{color:var(--fg-desc)}.text-accent{color:var(--accent)}',
  '.text-xs{font-size:10px;letter-spacing:.04em}.text-sm{font-size:11px}',
  '.text-lg{font-size:16px}.text-hero{font-size:42px;font-weight:200;line-height:1.1}',
  '.text-label{font-size:10px;text-transform:uppercase;letter-spacing:.07em;color:var(--fg-mute)}',
  '.mono{font-family:ui-monospace,"SF Mono","Fira Code",monospace;font-size:12px}',
  '.truncate{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}',
  '.row{display:flex;align-items:center;gap:8px}',
  '.col{display:flex;flex-direction:column;gap:6px}',
  '.fill{flex:1;min-width:0}.between{justify-content:space-between}.wrap{flex-wrap:wrap}',
  '.card{background:var(--bg-card);border-radius:var(--radius-sm);padding:10px 12px}',
  '.surface{background:var(--bg-deep);border-radius:var(--radius-sm);padding:10px 12px}',
  '.divider{height:1px;border:none;background:var(--line);margin:6px 0}',
  '.tag{display:inline-block;font-size:10px;background:var(--accent-soft);border-radius:3px;padding:1px 5px;color:var(--fg-mute)}',
  '.tag-accent{display:inline-block;font-size:10px;background:var(--accent);border-radius:3px;padding:1px 5px;color:var(--text-on-accent)}',
  '.bar{height:3px;border-radius:2px;background:var(--accent)}',
  '.accent-line{border-left:2px solid var(--accent-border);padding-left:8px}',
].join('');

function buildSrcdoc(content: string): string {
  const style = getComputedStyle(document.documentElement);
  const vars = THEME_VARS.map(v => `${v}:${style.getPropertyValue(v).trim()}`).join(';');
  return (
    `<!DOCTYPE html><html><head>` +
    `<meta http-equiv="Content-Security-Policy" ` +
    `content="default-src 'none'; style-src 'unsafe-inline' data:; img-src data:;">` +
    `<style>:root{${vars}}` +
    `html,body{height:100%}*{box-sizing:border-box;margin:0;padding:0}` +
    `body{background:transparent;color:var(--fg);font-size:13px;line-height:1.5;` +
    `font-family:system-ui,-apple-system,sans-serif;overflow-y:auto}` +
    `${EXT_UTILS_CSS}</style>` +
    `</head><body>${content}</body></html>`
  );
}

export default function ExtensionPreview({ result }: PreviewProps) {
  const version = useSyncExternalStore(subscribeVersion, () => cacheVersion);
  const [content, setContent] = useState<PreviewContent | null | undefined>(
    cache.get(result.id),
  );

  // Synchronously update state from cache before the browser paints - prevents
  // the one-frame flash where the previous result's content is visible while
  // result.id has already changed but the async useEffect hasn't fired yet.
  useLayoutEffect(() => {
    setContent(cache.has(result.id) ? cache.get(result.id) : undefined);
  }, [result.id, version]);

  // Async: fire the wasm invoke only for uncached results.
  useEffect(() => {
    if (cache.has(result.id)) return;
    if (!result.ext) return;
    let stale = false;
    invoke<PreviewContent | null>("extension_preview", { id: result.id, ext: result.ext })
      .then(c => {
        if (cache.size >= CACHE_MAX) cache.clear();
        cache.set(result.id, c);
        if (!stale) setContent(c);
      })
      .catch(() => {
        if (!stale) setContent(null);
      });
    return () => {
      stale = true;
    };
  }, [result.id, result.ext, version]);

  if (content == null) return <div className="preview-empty" />;

  switch (content.type) {
    case "markdown":
      return (
        <div className="ext-preview">
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{content.content}</ReactMarkdown>
        </div>
      );
    case "metadata":
      return (
        <div className="ext-preview">
          <table className="ext-preview-metadata">
            <tbody>
              {content.items.map((item, i) => (
                <tr key={i}>
                  <td className="ext-preview-label">{item.label}</td>
                  <td>{item.value}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case "image":
      return (
        <div className="ext-preview">
          <img
            className="ext-preview-image"
            src={`data:${content.mime};base64,${content.data_base64}`}
            alt={result.title}
          />
        </div>
      );
    case "list":
      return (
        <div className="ext-preview">
          {content.items.map((item, i) => (
            <div className="ext-preview-row" key={i}>
              <div className={item.mono ? "result-title ext-preview-row-mono" : "result-title"}>
                {item.title}
                {item.tag && <span className="ext-preview-row-tag">{item.tag}</span>}
              </div>
              {item.subtitle && <div className="result-subtitle">{item.subtitle}</div>}
            </div>
          ))}
        </div>
      );
    case "sections":
      return (
        <div className="ext-preview">
          {content.items.map((section, i) => (
            <div key={i} className="ext-preview-section">
              {section.heading && (
                <div className="ext-preview-section-heading">{section.heading}</div>
              )}
              <table className="ext-preview-section-table">
                <tbody>
                  {section.rows.map((row, j) =>
                    row.length === 1 ? (
                      <tr key={j}>
                        <td colSpan={2}>
                          <code className="ext-preview-section-solo">{row[0]}</code>
                        </td>
                      </tr>
                    ) : (
                      <tr key={j}>
                        <td><code className="ext-preview-section-cmd">{row[0]}</code></td>
                        <td className="ext-preview-section-desc">{row.slice(1).join("  ")}</td>
                      </tr>
                    )
                  )}
                </tbody>
              </table>
            </div>
          ))}
        </div>
      );
    case "code":
      return (
        <div className="ext-preview">
          <pre className="ext-preview-code">{content.content}</pre>
        </div>
      );
    case "html":
      return (
        <iframe
          className="ext-preview-html"
          sandbox=""
          srcDoc={buildSrcdoc(content.content)}
          title="extension preview"
        />
      );
    default:
      return <div className="preview-empty" />;
  }
}
