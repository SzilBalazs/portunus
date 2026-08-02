import { invoke } from "@tauri-apps/api/core";
import type { OfficeDoc } from "../../types";

// Rendered-document cache, keyed `path\0section\0terms` (the same shape as
// FilePreview's textHtmlKey) so a revisited file paints from memory instead of
// re-inflating its zip, re-decoding its media and re-running the highlighter.
//
// Two limits, not one. A count cap alone is not enough here: an office render is
// not a fixed-size bitmap - inlined media arrives base64 (+33% over the bytes on
// disk), and each html string is resident *three* times over while it is being
// shown (the cache entry, the `srcDoc` attribute, and the parsed document inside
// the frame). Twenty-four multi-megabyte sheets under a count-only cap would be a
// few hundred MB of resident strings.
const OFFICE_CACHE_CAP = 24;
const OFFICE_CACHE_BYTES = 24 * 1024 * 1024;

const docCache = new Map<string, OfficeDoc>();
const promiseCache = new Map<string, Promise<OfficeDoc>>();
let cachedBytes = 0;

/** Bytes an entry is charged. `html` dominates; the rest is a few hundred bytes. */
function docBytes(doc: OfficeDoc): number {
  return doc.html.length;
}

export function officeKey(path: string, section: number | null, terms: string[]): string {
  return `${path}\0${section ?? ""}\0${terms.join(" ")}`;
}

function store(key: string, doc: OfficeDoc) {
  docCache.set(key, doc);
  cachedBytes += docBytes(doc);
  // Evict oldest-first until *both* limits hold. Never evicts the entry just
  // stored, even if it alone exceeds the byte budget - the caller is about to
  // display it, and dropping it would only guarantee a re-render on the next
  // keystroke.
  while (docCache.size > 1 && (docCache.size > OFFICE_CACHE_CAP || cachedBytes > OFFICE_CACHE_BYTES)) {
    const oldest = docCache.keys().next().value as string | undefined;
    if (oldest === undefined || oldest === key) break;
    const evicted = docCache.get(oldest);
    if (evicted) cachedBytes -= docBytes(evicted);
    docCache.delete(oldest);
    promiseCache.delete(oldest);
  }
}

/** Synchronous cache read, for the render-time path (no fetch, no flash). */
export function peekOfficeDoc(key: string): OfficeDoc | undefined {
  return docCache.get(key);
}

/**
 * Render one section of an office document, deduped per key.
 *
 * The promise entry is removed on rejection so a transient failure (file being
 * written, permissions) doesn't poison the key for the rest of the session -
 * exactly the shape of FilePreview's getPdfUrl.
 */
export function getOfficeDoc(
  path: string,
  section: number | null,
  terms: string[],
): Promise<OfficeDoc> {
  const key = officeKey(path, section, terms);
  const cached = docCache.get(key);
  if (cached) return Promise.resolve(cached);
  if (!promiseCache.has(key)) {
    promiseCache.set(
      key,
      invoke<OfficeDoc>("render_office_doc", { path, section, terms })
        .then(doc => {
          store(key, doc);
          return doc;
        })
        .catch(e => {
          promiseCache.delete(key);
          throw e;
        }),
    );
  }
  return promiseCache.get(key)!;
}
