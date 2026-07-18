// Chord grammar: canonical string form "ctrl+alt+shift+<key>" - modifiers in
// that fixed order, exactly one key token, lowercase. The same tiny grammar is
// implemented in src-tauri/src/keybinds.rs (host-side clamp of untrusted
// extension shortcuts); keep the two in deliberate sync.
//
// Key tokens: a-z, 0-9, f1-f12, and the named set below. Named punctuation
// ("comma", not ",") avoids '+'-splitting ambiguity and TOML-quoting traps.
// Escape/backspace/delete/arrows/nav keys are not in the grammar at all -
// they are structural launcher keys.

import type { Shortcut } from "../actions/shortcut";

/** Named key tokens accepted besides letters, digits and f1-f12. */
const NAMED_KEYS = new Set([
  "enter", "tab", "space", "comma", "period", "slash", "semicolon", "quote",
  "backquote", "minus", "equal", "bracketleft", "bracketright", "backslash",
]);

/** Aliases folded into canonical tokens (chord strings and e.key values). */
const ALIASES: Record<string, string> = {
  control: "ctrl", return: "enter", spacebar: "space", " ": "space",
  ",": "comma", ".": "period", "/": "slash", ";": "semicolon", "'": "quote",
  "`": "backquote", "-": "minus", "=": "equal", "[": "bracketleft",
  "]": "bracketright", "\\": "backslash",
};

/** e.key value a named token matches (matchesShortcut compares e.key unless
 *  `code` is set); punctuation and space match by physical e.code instead. */
const NAMED_CODE: Record<string, string> = {
  space: "Space", comma: "Comma", period: "Period", slash: "Slash",
  semicolon: "Semicolon", quote: "Quote", backquote: "Backquote",
  minus: "Minus", equal: "Equal", bracketleft: "BracketLeft",
  bracketright: "BracketRight", backslash: "Backslash",
};

const isLetter = (t: string) => /^[a-z]$/.test(t);
const isDigit = (t: string) => /^[0-9]$/.test(t);
const isFKey = (t: string) => /^f([1-9]|1[0-2])$/.test(t);
const isKeyToken = (t: string) => isLetter(t) || isDigit(t) || isFKey(t) || NAMED_KEYS.has(t);

interface Parsed {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  key: string;
}

function parseTokens(chord: string): Parsed | null {
  if (!chord || chord.length > 32) return null;
  const tokens = chord.trim().toLowerCase().split("+").map(t => ALIASES[t.trim()] ?? t.trim());
  const p: Parsed = { ctrl: false, alt: false, shift: false, key: "" };
  for (const t of tokens) {
    if (t === "ctrl" || t === "alt" || t === "shift") {
      const mod = t as "ctrl" | "alt" | "shift";
      if (p[mod]) return null; // duplicate modifier
      p[mod] = true;
    } else if (t === "meta" || t === "super" || t === "cmd") {
      return null; // meta is never accepted (matchesShortcut rejects metaKey)
    } else if (isKeyToken(t)) {
      if (p.key) return null; // two key tokens
      p.key = t;
    } else {
      return null;
    }
  }
  return p.key ? p : null;
}

/** Canonical chord string, or null if unparseable. */
export function canonicalChord(chord: string): string | null {
  const p = parseTokens(chord);
  if (!p) return null;
  const parts: string[] = [];
  if (p.ctrl) parts.push("ctrl");
  if (p.alt) parts.push("alt");
  if (p.shift) parts.push("shift");
  parts.push(p.key);
  return parts.join("+");
}

/** Parse a chord string into the runtime `Shortcut`. Ctrl+letter/digit chords
 *  carry an e.code fallback (used only when WebKitGTK mangles e.key into a
 *  control-code name — see matchesShortcut); punctuation and space match by
 *  e.code, since their e.key is never a printable [a-z0-9]. */
export function parseChord(chord: string): Shortcut | null {
  const p = parseTokens(chord);
  if (!p) return null;
  const s: Shortcut = { key: p.key };
  if (p.ctrl) s.ctrl = true;
  if (p.alt) s.alt = true;
  if (p.shift) s.shift = true;
  if (NAMED_CODE[p.key]) s.code = NAMED_CODE[p.key];
  else if (p.ctrl && isLetter(p.key)) s.code = `Key${p.key.toUpperCase()}`;
  else if (p.ctrl && isDigit(p.key)) s.code = `Digit${p.key}`;
  return s;
}

/** Badge tokens for a chord string: "ctrl+shift+q" → ["ctrl","shift","q"].
 *  Same shape as shortcutParts() so kbd renderers are interchangeable. */
export function chordParts(chord: string): string[] {
  return chord.split("+");
}

/** Canonical chord string from a keydown, or null for modifier-only presses,
 *  IME composition, meta chords, and unmappable keys. Under ctrl a letter/digit
 *  resolves from the layout-produced e.key when it is printable (so QWERTZ/AZERTY
 *  map correctly); only when WebKitGTK has mangled e.key into a control-code name
 *  (Ctrl+H→Backspace, Ctrl+M→Enter, …) do we fall back to physical e.code. */
export function eventToChord(e: KeyboardEvent): string | null {
  if (e.isComposing || e.metaKey) return null;
  const raw = e.key;
  if (raw === "Control" || raw === "Alt" || raw === "Shift" || raw === "Meta") return null;
  let key: string | null = null;
  if (e.ctrlKey) {
    const k = raw.toLowerCase();
    if (isLetter(k) || isDigit(k)) key = k;
    else {
      const m = /^Key([A-Z])$/.exec(e.code);
      const d = /^Digit([0-9])$/.exec(e.code);
      if (m) key = m[1].toLowerCase();
      else if (d) key = d[1];
    }
  }
  if (!key) {
    const t = ALIASES[raw.toLowerCase()] ?? raw.toLowerCase();
    if (isKeyToken(t)) key = t;
  }
  if (!key) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("ctrl");
  if (e.altKey) parts.push("alt");
  if (e.shiftKey) parts.push("shift");
  parts.push(key);
  return parts.join("+");
}

/** Recorder-facing validation. `allowPlainTab` is the builtin:contents
 *  exception (Tab is its shipped default). Reserved chords are also dropped
 *  by the host clamp and never enter the runtime maps. */
export function validateChord(
  chord: string,
  opts?: { allowPlainTab?: boolean },
): { ok: true } | { ok: false; reason: string } {
  const p = parseTokens(chord);
  if (!p) return { ok: false, reason: "Unrecognized key" };
  const mods = p.ctrl || p.alt;
  if (p.key === "enter") {
    if (!p.ctrl && !p.alt && !p.shift) return { ok: false, reason: "Enter launches the result" };
    return { ok: true };
  }
  if (p.key === "tab") {
    if (!p.ctrl && !p.alt && !p.shift && !opts?.allowPlainTab)
      return { ok: false, reason: "Reserved by the launcher" };
    return { ok: true };
  }
  if (p.alt && !p.ctrl && !p.shift && isDigit(p.key))
    return { ok: false, reason: "Alt+digits jump to results" };
  if (!mods && !isFKey(p.key))
    return { ok: false, reason: "Add Ctrl or Alt - plain keys type into the search bar" };
  return { ok: true };
}

/** True when the chord may never be bound (recorder + runtime agreement). */
export function isReservedChord(chord: string, opts?: { allowPlainTab?: boolean }): boolean {
  return !validateChord(chord, opts).ok;
}

/** Chords that shadow common input editing - recorder soft-warns, never blocks. */
export const EDITING_CHORDS = new Set(["ctrl+a", "ctrl+x", "ctrl+v", "ctrl+z"]);
