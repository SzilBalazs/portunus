import { useLayoutEffect, useRef } from "react";
import { highlightInElement, markInElement, focusBestCluster } from "../highlight";

/**
 * Marks search terms inside the returned ref's subtree and scrolls the densest
 * section (most distinct terms) into view. The caller must remount the
 * highlighted subtree (via a `key`) whenever content or terms change, so the
 * effect always runs on clean, React-untouched DOM.
 *
 * Marking is synchronous whenever the subtree's words are already keyed - which
 * is the normal case, because the previews warm the keys (`warmHighlightKeys`)
 * before committing their content. That is what keeps marks and content in the
 * same paint: switching results used to swap the text first and pop the marks in
 * a frame or more later, once the keying round-trip resolved.
 *
 * The async path remains for the uncommon miss (evicted keys, a subtree whose
 * source text the caller never warmed); there the remount guarantees a
 * late-resolving highlight only ever mutates the current DOM.
 */
export function useTermHighlight<T extends HTMLElement>(terms: string[], dep: unknown) {
  const ref = useRef<T>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || !terms.length) return;
    const sync = markInElement(el, terms);
    if (sync) {
      sync.first && focusBestCluster(el)?.scrollIntoView({ block: "center" });
      return;
    }
    let cancelled = false;
    highlightInElement(el, terms, () => cancelled).then(() => {
      if (cancelled) return;
      focusBestCluster(el)?.scrollIntoView({ block: "center" });
    });
    return () => {
      cancelled = true;
    };
  }, [dep, terms]);
  return ref;
}
