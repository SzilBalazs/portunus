import { useEffect, useRef, useState } from "react";
import type { OfficeDoc } from "../../types";
import { getOfficeDoc, officeKey, peekOfficeDoc } from "./officeCache";

/** Debounce before invoking the renderer, matching the text/pdf previews: a file
 *  merely arrowed past must not pay for a zip inflate + media decode. */
const RENDER_DEBOUNCE_MS = 40;

export interface OfficeHtmlState {
  /** The document to show. Holds the *previous* file's document while a new one
   *  renders, so the frame never blanks; null only before the first ever result. */
  doc: OfficeDoc | null;
  /** Renderer error for the current file (e.g. "no HTML renderer yet"). */
  error: string | null;
  /** A render is outstanding for the current key. */
  loading: boolean;
}

/**
 * Resolves the rendered document for (path, section, terms).
 *
 * Two things here are load-bearing:
 *
 *  - The cache is read *in the state initializer* and again during render, never
 *    in an effect. An effect runs after commit, so the render that changes `path`
 *    commits the new path paired with the previous document; the frame would
 *    navigate to a document nobody asked for and then be corrected, and whichever
 *    render resolved first would paint. Reconciling during render makes React
 *    re-render before committing, so the mismatched pair never reaches an effect.
 *    (Same reasoning as PdfPreview's page reconciliation in FilePreview.tsx.)
 *  - A stale document is kept on screen rather than cleared. Blanking is the one
 *    thing an iframe host cannot afford: `srcDoc=null` is an extra navigation
 *    each way, i.e. two guaranteed white flashes.
 */
export function useOfficeHtml(
  path: string,
  section: number | null,
  terms: string[],
): OfficeHtmlState {
  const termsKey = terms.join(" ");
  const key = officeKey(path, section, terms);
  // The effect depends on `termsKey` (a stable string) rather than on the array
  // identity, so it reads the array itself through a ref.
  const termsRef = useRef(terms);
  termsRef.current = terms;

  const [doc, setDoc] = useState<OfficeDoc | null>(() => peekOfficeDoc(key) ?? null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(() => peekOfficeDoc(key) === undefined);

  // Render-time reconciliation: adopt a cache hit for a new path/section straight
  // away, and drop a previous file's error so the stale card can't outlive it.
  // Only on a path/section change - a same-file term refetch (i.e. typing in a
  // content search) keeps the current document up between keystrokes.
  const identityRef = useRef(`${path}\0${section ?? ""}`);
  const identity = `${path}\0${section ?? ""}`;
  if (identityRef.current !== identity) {
    identityRef.current = identity;
    const hit = peekOfficeDoc(key);
    if (hit) setDoc(hit);
    setError(null);
    setLoading(hit === undefined);
  }

  useEffect(() => {
    const cached = peekOfficeDoc(key);
    if (cached) {
      setDoc(cached);
      setError(null);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    const t = setTimeout(() => {
      getOfficeDoc(path, section, termsRef.current)
        .then(d => {
          if (cancelled) return;
          setDoc(d);
          setError(null);
          setLoading(false);
        })
        .catch(e => {
          if (cancelled) return;
          // Drop the held document too. Holding a stale one is only ever a way to
          // bridge a render; once this file is known to have no rendering, keeping
          // the previous file's content on screen would label it as this one.
          setDoc(null);
          setError(typeof e === "string" ? e : String(e));
          setLoading(false);
        });
    }, RENDER_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(t);
    };
    // termsKey stands in for the terms array (stable string identity).
  }, [key, path, section, termsKey]);

  return { doc, error, loading };
}
