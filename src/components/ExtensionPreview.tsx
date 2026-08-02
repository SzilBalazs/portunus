import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { PreviewProps } from "../providers/registry";
import type { PreviewChunk, PreviewContent } from "../types";
import { useTauriListener } from "../hooks/useTauriListener";
import { buildSrcdoc } from "../srcdoc";
import MarkdownView from "./MarkdownView";

// Per-id preview cache so flipping between results doesn't re-invoke the
// extension. Cleared wholesale when it grows - previews are tiny and queries
// are short-lived, an LRU would be overkill.
const cache = new Map<string, PreviewContent | null>();
const CACHE_MAX = 100;

// Extension reloads (Rescan button, `portunus --reload-extensions`) emit
// extensions-reloaded after swapping instances - drop cached previews and bump
// the version so even the currently-shown preview refetches (its result id is
// unchanged, so the id-keyed effect alone wouldn't rerun).
// NOT search-invalidated: that fires on every file/content watcher event, which
// would blank and refetch the preview on unrelated filesystem churn (flash).
let cacheVersion = 0;
const versionListeners = new Set<() => void>();
void listen("extensions-reloaded", () => {
  cache.clear();
  cacheVersion++;
  versionListeners.forEach(l => l());
});
const subscribeVersion = (cb: () => void) => {
  versionListeners.add(cb);
  return () => void versionListeners.delete(cb);
};

// Correlates a preview invoke with its streamed `extension-preview-chunk`
// events; module-level so it survives remounts.
let previewRequestCounter = Date.now();

/**
 * Renders the declarative preview an extension returned for the selected
 * result. Extensions never ship UI - they return data (markdown, metadata,
 * image, list) and this component renders it with the host's own widgets.
 *
 * Markdown goes through the shared <MarkdownView> (the same renderer the file
 * previews use), which sanitizes embedded HTML - scripts/handlers/javascript:
 * URLs are stripped. The `html` type still renders in a sandboxed iframe
 * (buildSrcdoc, src/srcdoc.ts) - it's opaque host-authored HTML, not markdown.
 */
export default function ExtensionPreview({ result }: PreviewProps) {
  const version = useSyncExternalStore(subscribeVersion, () => cacheVersion);
  const [content, setContent] = useState<PreviewContent | null | undefined>(
    cache.get(result.id),
  );
  const requestIdRef = useRef(0);

  // Synchronously update state from cache before the browser paints - prevents
  // the one-frame flash where the previous result's content is visible while
  // result.id has already changed but the async useEffect hasn't fired yet.
  useLayoutEffect(() => {
    setContent(cache.has(result.id) ? cache.get(result.id) : undefined);
  }, [result.id, version]);

  // Streamed intermediate content (LLM tokens, slow APIs): each chunk
  // replaces the render wholesale. Stale chunks are filtered by request id;
  // only the invoke's final resolution is cached.
  useTauriListener<PreviewChunk>("extension-preview-chunk", chunk => {
    if (chunk.request_id === requestIdRef.current) setContent(chunk.content);
  });

  // Async: fire the wasm invoke only for uncached results.
  useEffect(() => {
    if (cache.has(result.id)) return;
    if (!result.ext) return;
    let stale = false;
    const requestId = ++previewRequestCounter;
    requestIdRef.current = requestId;
    invoke<PreviewContent | null>("extension_preview", { id: result.id, ext: result.ext, requestId, command: result.ext_command ?? null })
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
      // Selection moved with the stream still running - abort it backend-side
      // (the next preview call would otherwise queue behind it).
      if (requestIdRef.current === requestId) {
        invoke("extension_preview_cancel", { id: result.id }).catch(() => {});
      }
    };
  }, [result.id, result.ext, version]);

  if (content == null) return <div className="preview-empty" />;

  switch (content.type) {
    case "markdown":
      // Same scroll container + renderer as the file previews, so extension
      // markdown looks identical to a previewed .md file.
      return (
        <div className="text-preview-wrap" data-selectable>
          <MarkdownView source={content.content} />
        </div>
      );
    case "metadata":
      return (
        <div className="ext-preview" data-selectable>
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
        <div className="ext-preview" data-selectable>
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
        <div className="ext-preview" data-selectable>
          <pre className="ext-preview-code">{content.content}</pre>
        </div>
      );
    case "html":
      return (
        <div className="ext-preview-html-wrap">
          <iframe
            className="ext-preview-html"
            sandbox=""
            srcDoc={buildSrcdoc(content.content)}
            title="extension preview"
          />
        </div>
      );
    default:
      return <div className="preview-empty" />;
  }
}
