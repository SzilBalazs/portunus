// The remappable built-in launcher chords and their shipped defaults. This is
// the single source of truth: the keydown dispatch (via the keybinds store),
// FooterHints badges, and the Settings Keybinds section all read it. Fixed
// entries are structural keys shown in Settings for completeness but locked.

export interface BuiltinBinding {
  /** Target id: "builtin:<name>"; the [keybinds.builtin] key is the bare name. */
  id: string;
  title: string;
  hint: string;
  /** Canonical chord strings; a [keybinds.builtin] entry replaces all of them. */
  defaults: string[];
  /** Structural key - listed for discoverability, never remappable. */
  fixed?: boolean;
}

export const BUILTIN_BINDINGS: BuiltinBinding[] = [
  { id: "builtin:quick-look", title: "Quick Look", hint: "Peek at the selected result", defaults: ["shift+enter"] },
  { id: "builtin:action-panel", title: "Actions panel", hint: "All actions for the selected result", defaults: ["alt+enter", "ctrl+k"] },
  { id: "builtin:contents", title: "Contents mode", hint: "Toggle full-text search inside files", defaults: ["tab"] },
  { id: "builtin:pin", title: "Pin result", hint: "Keep the result on top for this query", defaults: ["ctrl+p"] },
  { id: "builtin:copy", title: "Copy", hint: "Copy the result or the selected preview text", defaults: ["ctrl+c"] },
  { id: "builtin:search-selection", title: "Search selection", hint: "Start a new search from the selected text", defaults: ["ctrl+f"] },
  { id: "builtin:select-mode", title: "Select text", hint: "Keyboard text selection in the preview", defaults: ["ctrl+s"] },
  { id: "builtin:highlight", title: "Match highlight", hint: "Toggle matched-term highlighting (Contents mode)", defaults: ["ctrl+h"] },
  { id: "builtin:launch", title: "Open result", hint: "Launch the selected result", defaults: ["enter"], fixed: true },
  { id: "builtin:navigate", title: "Navigate results", hint: "Move the selection", defaults: ["up", "down"], fixed: true },
  { id: "builtin:jump", title: "Jump to result", hint: "Launch by row number", defaults: ["alt+1…9"], fixed: true },
  { id: "builtin:escape", title: "Escape", hint: "Unwind overlay, query, mode, then hide", defaults: ["escape"], fixed: true },
];

/** The shared copy-chord family: remapping builtin:copy moves the default
 *  chord of these provider actions too (an individual [keybinds.actions]
 *  override still wins). */
export const COPY_ACTION_IDS = new Set(["file:copy-path", "calc:copy", "dict:copy"]);
