// Matched-term highlighting for content-search (`!query`) previews.
//
// The query terms are derived the same way the Rust content provider tokenizes
// them (see src-tauri/src/providers/content.rs), and matching is word-prefix /
// case-insensitive to mirror the porter stemmer used by the FTS index — so
// searching `run` highlights `running`.

const HL_CLASS = "preview-hl";

/** Tokenizes a `!query` into the terms the content index matched on. */
export function deriveContentTerms(query: string): string[] {
  const q = query.trimStart().replace(/^!/, "").trim();
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
