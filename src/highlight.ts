// Matched-term highlighting for content-mode (full-text) previews.
//
// The content index tokenizes with FTS5 `porter unicode61`. To highlight and jump
// to exactly what it matched, this module keys words the SAME way - but instead of
// re-implementing Porter in JS, it delegates to the backend `content_match_keys`
// command (which wraps `content_match::match_key`). Two words match iff they share
// a key. Keys are cached per-word for the session so repeated words / re-opens skip
// the round-trip. Because keying is async, the highlight functions are async too;
// `focusBestCluster` stays sync by reading the key stamped on each `<mark>`.

import { invoke } from "@tauri-apps/api/core";

const HL_CLASS = "preview-hl";

/** word -> match key, session-lived so repeats and re-opens skip the backend call.
 * Bounded so browsing many large documents can't grow it without limit; eviction is
 * insertion-order (Map keeps it) - approximate LRU, and evicted words are simply
 * re-fetched on demand. */
const keyCache = new Map<string, string>();
const KEY_CACHE_MAX = 20_000;

/** A token and its byte offsets within the source string. */
export interface Token {
  start: number;
  end: number;
  word: string;
}

/** Splits text into `[\p{L}\p{N}]+` runs (apostrophe/punctuation separate), mirroring
 * unicode61's token boundaries. Offsets are into `text`. */
export function tokenize(text: string): Token[] {
  // Keep accented words whole regardless of how the extractor split them: \p{M}
  // for NFD combining marks, plus the non-ASCII spacing accents pdfium emits
  // (U+00A8/AF/B4/B8 and the Spacing Modifier Letters block). The backend strips
  // these when keying. Mirrors content_match::tokenize / is_diacritic.
  const re = /[\p{L}\p{N}\p{M}¨¯´¸ʰ-˿]+/gu;
  const out: Token[] = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    out.push({ start: m.index, end: m.index + m[0].length, word: m[0] });
  }
  return out;
}

/** Tokenizes a content-mode query into the candidate terms to highlight. */
export function deriveContentTerms(query: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const { word } of tokenize(query)) {
    if (word.length < 2) continue; // single-char terms are highlight noise
    const lower = word.toLowerCase();
    if (seen.has(lower)) continue; // dedup; backend collapses casing anyway
    seen.add(lower);
    out.push(word);
  }
  return out;
}

/** Ensures match keys for `words` are cached, with one batched backend call for the
 * misses (deduped). Safe to call repeatedly; cached words cost nothing. */
export async function ensureKeys(words: Iterable<string>): Promise<void> {
  const miss: string[] = [];
  const seen = new Set<string>();
  for (const w of words) {
    if (!keyCache.has(w) && !seen.has(w)) {
      seen.add(w);
      miss.push(w);
    }
  }
  if (!miss.length) return;
  const keys = await invoke<string[]>("content_match_keys", { words: miss });
  miss.forEach((w, i) => keyCache.set(w, keys[i] ?? ""));
  while (keyCache.size > KEY_CACHE_MAX) {
    const oldest = keyCache.keys().next().value;
    if (oldest === undefined) break;
    keyCache.delete(oldest);
  }
}

/** Cached match key for `word`, or undefined if not fetched yet (call `ensureKeys`). */
export function keyOf(word: string): string | undefined {
  return keyCache.get(word);
}

/** Builds the query-key set from already-cached keys (sync; call `ensureKeys` first). */
function querySet(terms: string[]): Set<string> {
  const set = new Set<string>();
  for (const t of terms) {
    const k = keyOf(t);
    if (k) set.add(k);
  }
  return set;
}

/** The set of match keys for `terms` (fetches them first). */
export async function loadQueryKeys(terms: string[]): Promise<Set<string>> {
  await ensureKeys(terms);
  return querySet(terms);
}

/** True if any word in `text` keys to a query key in `qkeys`. Sync: requires the
 * words to have been keyed already (e.g. via `ensureKeys`). */
export function cellMatches(text: string, qkeys: Set<string>): boolean {
  if (!qkeys.size) return false;
  for (const { word } of tokenize(text)) {
    const k = keyOf(word);
    if (k !== undefined && qkeys.has(k)) return true;
  }
  return false;
}

/** The markable text nodes of a subtree, with their tokens. */
interface MarkPlan {
  nodes: Text[];
  tokensByNode: Token[][];
}

/** Collects the text nodes to mark inside `el` (skipping already-marked text and
 * non-content elements) plus their tokens. Read-only - the caller mutates. */
function planMarks(el: HTMLElement): MarkPlan {
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
  const nodes: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) nodes.push(n as Text);
  return { nodes, tokensByNode: nodes.map((n) => tokenize(n.nodeValue ?? "")) };
}

/** Every word the plan needs keyed, query terms included. */
function planWords(plan: MarkPlan, terms: string[]): string[] {
  const out = terms.slice();
  for (const toks of plan.tokensByNode) for (const t of toks) out.push(t.word);
  return out;
}

/** True once every word in the plan (and every term) has a cached key, i.e. marking
 * can run without a backend round-trip. */
function planIsKeyed(plan: MarkPlan, terms: string[]): boolean {
  for (const w of planWords(plan, terms)) if (!keyCache.has(w)) return false;
  return true;
}

/** Replaces matched words with `<mark>` elements. Requires cached keys. */
function applyMarks(plan: MarkPlan, qkeys: Set<string>): HTMLElement | null {
  let first: HTMLElement | null = null;
  plan.nodes.forEach((node, ni) => {
    const text = node.nodeValue ?? "";
    const hits = plan.tokensByNode[ni].filter((t) => {
      const k = keyOf(t.word);
      return k !== undefined && qkeys.has(k);
    });
    if (!hits.length) return;

    const frag = document.createDocumentFragment();
    let last = 0;
    for (const h of hits) {
      if (h.start > last) frag.appendChild(document.createTextNode(text.slice(last, h.start)));
      const mark = document.createElement("mark");
      mark.className = HL_CLASS;
      mark.textContent = text.slice(h.start, h.end);
      mark.dataset.hlkey = keyOf(h.word) ?? "";
      frag.appendChild(mark);
      if (!first) first = mark;
      last = h.end;
    }
    if (last < text.length) frag.appendChild(document.createTextNode(text.slice(last)));
    node.parentNode?.replaceChild(frag, node);
  });
  return first;
}

/**
 * Synchronous marking, for the common case where the subtree's words are already
 * keyed (the preview warmed them before committing its content - see
 * `warmHighlightKeys`). Returns `{ first }` when it ran, or `null` when some word
 * is unkeyed and the caller must fall back to `highlightInElement`. Running sync
 * matters: called from a layout effect it puts the marks in the same paint as the
 * content, instead of a frame or more later.
 */
export function markInElement(el: HTMLElement, terms: string[]): { first: HTMLElement | null } | null {
  if (!terms.length) return { first: null };
  const plan = planMarks(el);
  if (!planIsKeyed(plan, terms)) return null;
  const qkeys = querySet(terms);
  if (!qkeys.size) return { first: null };
  return { first: applyMarks(plan, qkeys) };
}

/**
 * Pre-keys `terms` plus every word in `text` so a later `markInElement` over the
 * rendered form of that text can run synchronously. Callers await this before
 * committing the content, so content and marks land together.
 */
export async function warmHighlightKeys(text: string, terms: string[]): Promise<void> {
  if (!terms.length) return;
  const words = terms.slice();
  for (const t of tokenize(text)) words.push(t.word);
  await ensureKeys(words);
}

/**
 * Wraps matched words in `<mark class="preview-hl">` inside `el`, walking text nodes
 * so it works over highlight.js / ReactMarkdown output (whose HTML can't be safely
 * string-replaced). A word matches when its key is among the query keys. Each mark is
 * stamped with `data-hlkey` so `focusBestCluster` can group by distinct key without
 * re-keying. Returns the first mark element, for scrolling.
 */
export async function highlightInElement(
  el: HTMLElement,
  terms: string[],
  shouldCancel?: () => boolean,
): Promise<HTMLElement | null> {
  if (!terms.length) return null;

  // Collect text nodes first; we mutate the DOM as we go.
  const plan = planMarks(el);

  // Key the query terms plus every word in the subtree in one batched call.
  await ensureKeys(planWords(plan, terms));
  // The await is a backend round-trip; bail before touching the DOM if the caller
  // (e.g. a preview that navigated away) cancelled in the meantime.
  if (shouldCancel?.()) return null;

  const qkeys = querySet(terms);
  if (!qkeys.size) return null;
  return applyMarks(plan, qkeys);
}

/**
 * Of all `mark.preview-hl` in `el`, returns the one starting the section that covers
 * the most DISTINCT keys within PROXIMITY_PX vertically; earliest wins ties. Reads the
 * `data-hlkey` stamped by `highlightInElement` (no re-keying). Pairs with the backend's
 * coverage-window selection so the scroll lands on the densest section.
 */
export function focusBestCluster(el: HTMLElement): HTMLElement | null {
  const PROXIMITY_PX = 240;
  const marks = Array.from(
    el.querySelectorAll<HTMLElement>(`mark.${HL_CLASS}`),
  ).map((m) => ({ el: m, top: m.offsetTop, key: m.dataset.hlkey ?? "" }));
  if (!marks.length) return null;

  let best = marks[0].el;
  let bestDistinct = -1;
  let bestTop = Infinity;
  for (const m of marks) {
    const seen = new Set<string>();
    for (const o of marks) {
      if (Math.abs(o.top - m.top) <= PROXIMITY_PX && o.key) seen.add(o.key);
    }
    if (seen.size > bestDistinct || (seen.size === bestDistinct && m.top < bestTop)) {
      bestDistinct = seen.size;
      bestTop = m.top;
      best = m.el;
    }
  }
  return best;
}
