// Launcher-side keybinds store: the effective chord maps built from the
// [keybinds] config overlaid on code defaults (builtins.ts) and catalog
// defaults (command default_shortcut). Module-level cache + snapshot, same
// idiom as commands/store.ts. Sources: one lazy get_config fetch, the
// payload-carrying `keybinds-changed` event (Settings save or a manual
// config.toml edit via the watcher), and the command catalog store.

import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ChordList, Config, KeybindsConfig } from "../types";
import type { Shortcut } from "../actions/shortcut";
import { matchesShortcut } from "../actions/shortcut";
import { canonicalChord, parseChord } from "./chord";
import { BUILTIN_BINDINGS, COPY_ACTION_IDS } from "./builtins";
import { getCommands, subscribeCommands } from "../commands/store";

export interface KeybindSnapshot {
  /** Action target id → canonical chords; empty array = explicitly cleared.
   *  Absent = no override (shipped default applies). */
  actionOverrides: Map<string, string[]>;
  /** Canonical chord → command id. User bindings ∪ catalog default_shortcuts
   *  (user wins); entries whose command is missing from the live catalog are
   *  dropped so stale bindings never swallow keys. */
  commandChords: Map<string, string>;
  /** Canonical chord → builtin id (defaults overlaid with [keybinds.builtin]). */
  builtinChords: Map<string, string>;
  /** Builtin id → effective Shortcuts (chord-aware guards + kbd badges). */
  builtinShortcuts: Map<string, Shortcut[]>;
}

const EMPTY_KEYBINDS: KeybindsConfig = { builtin: {}, commands: {}, actions: {} };

let keybinds: KeybindsConfig = EMPTY_KEYBINDS;
let snapshot: KeybindSnapshot = build();
let started = false;
const listeners = new Set<() => void>();

/** ChordList → canonical chord array; unparseable entries dropped with a warning. */
function chords(list: ChordList | undefined): string[] {
  if (list === undefined) return [];
  const arr = typeof list === "string" ? (list === "" ? [] : [list]) : list;
  const out: string[] = [];
  for (const c of arr) {
    const canon = canonicalChord(c);
    if (canon) out.push(canon);
    else console.warn(`[keybinds] ignoring unparseable chord ${JSON.stringify(c)}`);
  }
  return out;
}

function build(): KeybindSnapshot {
  const actionOverrides = new Map<string, string[]>();
  for (const [id, list] of Object.entries(keybinds.actions)) {
    actionOverrides.set(id, chords(list));
  }

  const builtinChords = new Map<string, string>();
  const builtinShortcuts = new Map<string, Shortcut[]>();
  for (const b of BUILTIN_BINDINGS) {
    if (b.fixed) continue;
    const name = b.id.slice("builtin:".length);
    const override = keybinds.builtin[name];
    const effective = override !== undefined ? chords(override) : b.defaults;
    const shortcuts: Shortcut[] = [];
    for (const c of effective) {
      const s = parseChord(c);
      if (!s) continue;
      builtinChords.set(c, b.id);
      shortcuts.push(s);
    }
    builtinShortcuts.set(b.id, shortcuts);
  }

  // Catalog defaults first, user entries second (user wins); sorted by target
  // id so same-chord collisions resolve deterministically.
  const commandChords = new Map<string, string>();
  const catalog = getCommands();
  const live = new Set(catalog.map(c => c.id));
  const userBound = new Set(Object.keys(keybinds.commands));
  for (const cmd of [...catalog].sort((a, b) => a.id.localeCompare(b.id))) {
    if (!cmd.default_shortcut || userBound.has(cmd.id)) continue;
    const canon = canonicalChord(cmd.default_shortcut);
    if (canon && !commandChords.has(canon)) commandChords.set(canon, cmd.id);
  }
  for (const id of [...userBound].sort()) {
    if (!live.has(id)) continue;
    for (const c of chords(keybinds.commands[id])) {
      commandChords.set(c, id);
    }
  }

  return { actionOverrides, commandChords, builtinChords, builtinShortcuts };
}

function rebuild() {
  snapshot = build();
  listeners.forEach(l => l());
}

function ensureStarted() {
  if (started) return;
  started = true;
  invoke<Config>("get_config")
    .then(cfg => {
      keybinds = cfg.keybinds ?? EMPTY_KEYBINDS;
      rebuild();
    })
    .catch(() => {});
  void listen<KeybindsConfig>("keybinds-changed", e => {
    keybinds = e.payload ?? EMPTY_KEYBINDS;
    rebuild();
  });
  // Command catalog arrival/refresh changes default_shortcut availability.
  subscribeCommands(rebuild);
}

const subscribe = (cb: () => void) => {
  ensureStarted();
  listeners.add(cb);
  return () => void listeners.delete(cb);
};

/** Imperative snapshot for the keydown path (no React dependency). */
export function getKeybinds(): KeybindSnapshot {
  ensureStarted();
  return snapshot;
}

/** Reactive snapshot for badges and the action panel memo. */
export function useKeybinds(): KeybindSnapshot {
  return useSyncExternalStore(subscribe, () => snapshot);
}

/** Effective shortcut for a result action: user override ?? copy-family chord
 *  ?? the shipped default. `[]` override (cleared) → undefined (menu-only). */
export function effectiveActionShortcut(
  id: string,
  shipped: Shortcut | string | undefined,
): Shortcut | undefined {
  const snap = getKeybinds();
  const override = snap.actionOverrides.get(id);
  if (override !== undefined) {
    return override.length ? parseChord(override[0]) ?? undefined : undefined;
  }
  if (COPY_ACTION_IDS.has(id)) return snap.builtinShortcuts.get("builtin:copy")?.[0];
  return typeof shipped === "string" ? parseChord(shipped) ?? undefined : shipped;
}

/** Chord-aware guard for a built-in ("does this event hit builtin:copy?"). */
export function matchesBuiltin(e: KeyboardEvent, id: string): boolean {
  const shortcuts = getKeybinds().builtinShortcuts.get(id);
  return !!shortcuts?.some(s => matchesShortcut(e, s));
}
