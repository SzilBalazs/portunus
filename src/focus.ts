// Host focus custody.
//
// The launcher keeps focus on the search input at all times: App.tsx cancels
// every `mousedown` on the card that would move it, so keybinds work no matter
// where the pointer went. Sandboxed iframes are the one hole in that rule -
// their mousedown is dispatched inside *their* document, so the host handler
// never runs and the click quietly focuses the frame element, after which the
// window keydown listeners see nothing.
//
// A frame that notices it has focus asks the host to take it back through here.
// Module singleton rather than a prop chain because the frames sit several
// components below App and the answer ("who should hold focus right now?") is
// App's alone - QuickLook deliberately runs with the input blurred.

type FocusRestorer = () => void;

let restorer: FocusRestorer | null = null;

export const hostFocus = {
  /** App installs the one restorer. Returns the uninstall. */
  register(fn: FocusRestorer): () => void {
    restorer = fn;
    return () => {
      if (restorer === fn) restorer = null;
    };
  },

  /** Put focus back where the host wants it. No-op before App has registered. */
  restore(): void {
    restorer?.();
  },
};
