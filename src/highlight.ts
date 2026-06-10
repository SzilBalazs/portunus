// Matched-term highlighting for content-mode (full-text) previews.
//
// The query terms are derived the same way the Rust content provider tokenizes
// them (see src-tauri/src/providers/content.rs), and matching is word-prefix /
// case-insensitive to mirror the porter stemmer used by the FTS index - so
// searching `run` highlights `running`.

const HL_CLASS = "preview-hl";

/** Tokenizes a content-mode query into the terms the content index matched on. */
export function deriveContentTerms(query: string): string[] {
  const q = query.trim();
  if (!q) return [];
  return q
    // Same cleaning as content.rs: keep alphanumerics + apostrophe, split on the rest.
    .replace(/[^\p{L}\p{N}']+/gu, " ")
    .split(/\s+/)
    .filter((t) => t.length >= 2);
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Word-prefix, case-insensitive regex over all terms, or null if none. */
export function buildTermRegex(terms: string[]): RegExp | null {
  if (!terms.length) return null;
  const alt = terms.map(escapeRegex).join("|");
  return new RegExp(`\\b(?:${alt})[\\p{L}\\p{N}]*`, "giu");
}

/** True if any term word-prefix-matches the given cell/text. */
export function cellMatches(text: string, terms: string[]): boolean {
  const re = buildTermRegex(terms);
  return re ? re.test(text) : false;
}

/**
 * Wraps matched terms in `<mark class="preview-hl">` inside `el`, walking text
 * nodes so it works over highlight.js / ReactMarkdown output (whose HTML can't
 * be safely string-replaced). Returns the first mark element, for scrolling.
 */
export function highlightInElement(
  el: HTMLElement,
  terms: string[],
): HTMLElement | null {
  const re = buildTermRegex(terms);
  if (!re) return null;

  // Collect text nodes first; we mutate the DOM as we go.
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      if (!node.nodeValue || !node.nodeValue.trim()) return NodeFilter.FILTER_REJECT;
      const parent = node.parentElement;
      if (!parent) return NodeFilter.FILTER_REJECT;
      // Skip already-marked text and non-content elements.
      if (parent.closest(`mark.${HL_CLASS}`)) return NodeFilter.FILTER_REJECT;
      const tag = parent.tagName;
      if (tag === "SCRIPT" || tag === "STYLE") return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  const textNodes: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    textNodes.push(n as Text);
  }

  let first: HTMLElement | null = null;
  for (const node of textNodes) {
    const text = node.nodeValue ?? "";
    re.lastIndex = 0;
    if (!re.test(text)) continue;
    re.lastIndex = 0;

    const frag = document.createDocumentFragment();
    let last = 0;
    let m: RegExpExecArray | null;
    while ((m = re.exec(text)) !== null) {
      if (m.index > last) frag.appendChild(document.createTextNode(text.slice(last, m.index)));
      const mark = document.createElement("mark");
      mark.className = HL_CLASS;
      mark.textContent = m[0];
      frag.appendChild(mark);
      if (!first) first = mark;
      last = m.index + m[0].length;
      if (m[0].length === 0) re.lastIndex++; // guard against zero-width loops
    }
    if (last < text.length) frag.appendChild(document.createTextNode(text.slice(last)));
    node.parentNode?.replaceChild(frag, node);
  }
  return first;
}

/** Index of the first term that word-prefix-matches `text`, or -1. */
function termOf(text: string, terms: string[]): number {
  const w = text.toLowerCase();
  return terms.findIndex((t) => w.startsWith(t.toLowerCase()));
}

/**
 * Of all `mark.preview-hl` in `el`, returns the one starting the section that
 * covers the most DISTINCT terms within PROXIMITY_PX vertically; earliest wins ties.
 * Falls back to the first mark (or null). Pairs with the backend's coverage-window
 * selection so the scroll lands on the densest section, not the first lone term.
 */
export function focusBestCluster(
  el: HTMLElement,
  terms: string[],
): HTMLElement | null {
  if (!terms.length) return null;
  const PROXIMITY_PX = 240;
  const marks = Array.from(
    el.querySelectorAll<HTMLElement>(`mark.${HL_CLASS}`),
  ).map((m) => ({ el: m, top: m.offsetTop, term: termOf(m.textContent ?? "", terms) }));
  if (!marks.length) return null;

  let best = marks[0].el;
  let bestDistinct = -1;
  let bestTop = Infinity;
  for (const m of marks) {
    const seen = new Set<number>();
    for (const o of marks) {
      if (Math.abs(o.top - m.top) <= PROXIMITY_PX && o.term >= 0) seen.add(o.term);
    }
    if (seen.size > bestDistinct || (seen.size === bestDistinct && m.top < bestTop)) {
      bestDistinct = seen.size;
      bestTop = m.top;
      best = m.el;
    }
  }
  return best;
}
