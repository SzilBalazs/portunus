// Module singleton (same idiom as `selection` and `pdfView`) that lets QuickLook
// scroll an office preview it cannot reach.
//
// Every other preview scrolls because QuickLook can find a DOM scroller inside
// it. An office document's scroller lives in another document, so the keys have
// to be forwarded over postMessage instead - and the component that owns the
// frame handle is the only thing that can do the forwarding.

import type { HostMessage } from "./protocol";

/** Sends a message to a mounted office frame; false when it has none live yet. */
export type OfficeScrollHandler = (msg: HostMessage) => boolean;

// A stack, not a single slot: the side-panel preview stays mounted underneath an
// open QuickLook, so both are registered at once. The QuickLook copy mounts
// second and therefore wins - the same last-one-wins arbitration pdfQuicklookMounted
// does for PDF page-flips.
const handlers: OfficeScrollHandler[] = [];

export const officeScroll = {
  register(fn: OfficeScrollHandler): () => void {
    handlers.push(fn);
    return () => {
      const i = handlers.lastIndexOf(fn);
      if (i >= 0) handlers.splice(i, 1);
    };
  },

  /**
   * True when `fn` is the handler messages route to.
   *
   * Both the side panel and the Quicklook overlay mount a preview of the same
   * document, so both install a window-level zoom listener. Without this the
   * two would each apply a step and every ctrl+= would zoom twice.
   */
  isTop(fn: OfficeScrollHandler): boolean {
    return handlers.length > 0 && handlers[handlers.length - 1] === fn;
  },

  /**
   * Enter keyboard caret mode in the office frame on top (the select-mode chord,
   * for which there is no host-side `[data-selectable]` root to hand the engine).
   * False when no frame has a document live yet, so the chord falls through.
   */
  enterSelect(): boolean {
    const fn = handlers[handlers.length - 1];
    return fn ? fn({ type: "selEnter" }) : false;
  },

  /** True when an office frame consumed the key (so the caller preventDefaults). */
  handleKey(key: string, shift: boolean): boolean {
    const fn = handlers[handlers.length - 1];
    if (!fn) return false;
    const line = 80;
    switch (key) {
      case "ArrowDown": return fn({ type: "scrollBy", dy: line });
      case "ArrowUp": return fn({ type: "scrollBy", dy: -line });
      case "ArrowRight": return fn({ type: "scrollBy", dx: line });
      case "ArrowLeft": return fn({ type: "scrollBy", dx: -line });
      case "PageDown": return fn({ type: "scrollBy", pages: 1 });
      case "PageUp": return fn({ type: "scrollBy", pages: -1 });
      case " ": return fn({ type: "scrollBy", pages: shift ? -1 : 1 });
      case "Home": return fn({ type: "scrollTo", top: "start" });
      case "End": return fn({ type: "scrollTo", top: "end" });
      default: return false;
    }
  },
};
