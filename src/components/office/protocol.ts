// postMessage protocol between the host and a rendered office document.
//
// The frame's origin is opaque (`sandbox="allow-scripts"` without
// `allow-same-origin`), which means `e.origin` is the literal string "null" on
// every message and proves nothing. Identity is therefore established twice:
//
//   1. `e.source === frame.contentWindow` - the message came from a window we own.
//   2. `e.data.token === <the token baked into that document's srcdoc>` - it came
//      from *this* document, not a stale predecessor still finishing a task.
//
// Both checks are required. (1) alone lets a stale buffer's `ready` flip the
// host to the wrong document; (2) alone would trust any window that had somehow
// observed the token.

/** Host → frame. Every message is stamped with the document's token. */
export type HostMessage =
  /** Relative scroll. `pages` is in viewport-heights, resolved in-frame because
   *  the host does not know the document's client height. */
  | { type: "scrollBy"; dx?: number; dy?: number; pages?: number }
  /** Absolute scroll; "start"/"end" saturate. */
  | { type: "scrollTo"; top: number | "start" | "end" }
  /** Matched-term highlighting (Ctrl+H). A class flip, so it is instant. */
  | { type: "hl"; on: boolean }
  /** Reset to the top of the current section. */
  | { type: "section" }
  /** Reader zoom, multiplied into the launcher's own UI scale. */
  | { type: "zoom"; factor: number; requestId?: number };

/** Frame → host. */
export type FrameMessage =
  /** Parsed, and the best match already centred - the host's cue to reveal. */
  | { type: "ready" }
  /** The document's current text selection ("" when cleared). */
  | { type: "selection"; text: string }
  /** Zoom changed - either an ack for a host `zoom` (echoing its `requestId`) or
   *  an unsolicited report of an in-frame ctrl+wheel. */
  | { type: "zoomed"; factor: number; requestId?: number }
  /** Focus reached the frame; the host should take it back (see the focus-custody
   *  note in `officeBootstrap`). */
  | { type: "refocus" };

/** A frame message plus the envelope fields the bootstrap adds. */
export type TokenedFrameMessage = FrameMessage & { token: string };

/**
 * Narrows a raw `MessageEvent.data` to a frame message carrying `token`.
 *
 * Deliberately structural and shallow: this is untrusted input in the sense that
 * *anything* can postMessage into the host window, so nothing is read off the
 * value until both this and the `e.source` check have passed.
 */
export function isFrameMessage(data: unknown): data is TokenedFrameMessage {
  if (typeof data !== "object" || data === null) return false;
  const m = data as Record<string, unknown>;
  if (typeof m.token !== "string") return false;
  return (
    m.type === "ready" ||
    m.type === "selection" ||
    m.type === "zoomed" ||
    m.type === "refocus"
  );
}
