import { useEffect, useState, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { PreviewProps } from "../providers/registry";
import type { PreviewContent } from "../types";

// Per-id preview cache so flipping between results doesn't re-invoke the
// extension. Cleared wholesale when it grows — previews are tiny and queries
// are short-lived, an LRU would be overkill.
const cache = new Map<string, PreviewContent | null>();
const CACHE_MAX = 100;

// Extension reloads (Rescan button, `portunus --reload-extensions`) emit
// search-invalidated after swapping instances — drop cached previews and bump
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
 * result. Extensions never ship UI — they return data (markdown, metadata,
 * image, list) and this component renders it with the host's own widgets.
 * Raw HTML in markdown is NOT rendered (no rehype-raw) — extension content
 * stays inert.
 */
export default function ExtensionPreview({ result }: PreviewProps) {
  const version = useSyncExternalStore(subscribeVersion, () => cacheVersion);
  const [content, setContent] = useState<PreviewContent | null | undefined>(
    cache.get(result.id),
  );

  useEffect(() => {
    if (cache.has(result.id)) {
      setContent(cache.get(result.id));
      return;
    }
    setContent(undefined);
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
              <div className="result-title">{item.title}</div>
              {item.subtitle && <div className="result-subtitle">{item.subtitle}</div>}
            </div>
          ))}
        </div>
      );
    default:
      return <div className="preview-empty" />;
  }
}
