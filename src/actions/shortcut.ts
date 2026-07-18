// Structured keyboard shortcuts: declared once on an ActionDescriptor, they
// drive both the window-level chord dispatch and the action panel's kbd badge,
// so the handler and the displayed hint can never drift apart.

export interface Shortcut {
  /** KeyboardEvent.key, canonical lowercase: "enter", "c", "tab". */
  key: string;
  ctrl?: boolean;
  alt?: boolean;
  shift?: boolean;
  /** Physical-position fallback for when WebKitGTK mangles e.key into a
   *  control-code name (Ctrl+H→Backspace, Ctrl+M→Enter). e.code stays "KeyH".
   *  The layout-produced e.key is still preferred when it is printable, so
   *  QWERTZ/AZERTY letters and digits match by the char the user actually typed. */
  code?: string;
}

/** Prefer the layout char (e.key) when printable; fall back to physical e.code
 *  only for the mangled control-code combos and for punctuation/space (whose
 *  s.code is set from NAMED_CODE and whose e.key is never [a-z0-9]). */
const matchesKey = (e: KeyboardEvent, s: Shortcut): boolean => {
  const k = e.key.toLowerCase();
  if (s.code && !/^[a-z0-9]$/.test(k)) return e.code === s.code;
  return k === s.key;
};

export const matchesShortcut = (e: KeyboardEvent, s: Shortcut): boolean =>
  matchesKey(e, s) &&
  e.ctrlKey === !!s.ctrl &&
  e.altKey === !!s.alt &&
  e.shiftKey === !!s.shift &&
  !e.metaKey;

/** Badge tokens: {ctrl:true, key:"enter"} → ["ctrl", "enter"]. The renderer
 *  maps "enter" to <EnterIcon/> - same idiom as FooterHints. */
export function shortcutParts(s: Shortcut): string[] {
  const parts: string[] = [];
  if (s.ctrl) parts.push("ctrl");
  if (s.alt) parts.push("alt");
  if (s.shift) parts.push("shift");
  parts.push(s.key);
  return parts;
}
