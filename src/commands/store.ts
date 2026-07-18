// Launcher-side command catalog store: one lazy `list_commands` fetch shared
// by mode entry (Tab, --clipboard, entry launch) and chips. Mirrors the
// extension meta store: module-level cache + useSyncExternalStore, refetched
// on `extensions-reloaded` (extension commands change) and
// `search-invalidated` (built-in provider rebuilds change availability).

import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { CommandDescriptor } from "../types";

// Static fallbacks so Tab-into-contents and `portunus --clipboard` work even
// before the first fetch lands (or if it fails). Mirror the backend
// descriptors in clipboard.rs / content.rs.
export const FALLBACK_CONTENTS: CommandDescriptor = {
  id: "cmd:contents",
  title: "Search File Contents",
  chip: "Contents",
  subtitle: "Full-text search inside files",
  source: { type: "builtin" },
  mode_kind: "scope",
  keywords: ["contents", "grep", "search", "text", "fulltext"],
  placeholder: "Search file contents…",
  min_query_len: 2,
  result_kind: "file",
  route: { type: "builtin", provider_id: "content" },
};

export const FALLBACK_CLIPBOARD: CommandDescriptor = {
  id: "cmd:clipboard",
  title: "Clipboard History",
  chip: "Clipboard",
  subtitle: "Browse, paste and manage copied items",
  source: { type: "builtin" },
  mode_kind: "scope",
  keywords: ["clip", "clipboard", "paste", "history", "copy"],
  placeholder: "Search clipboard history…",
  min_query_len: 0,
  result_kind: "clipboard",
  route: { type: "ui_takeover" },
};

let cache: CommandDescriptor[] = [];
let started = false;
const listeners = new Set<() => void>();

function refetch() {
  invoke<CommandDescriptor[]>("list_commands")
    .then(list => {
      cache = list;
      listeners.forEach(l => l());
    })
    .catch(() => {});
}

function ensureStarted() {
  if (started) return;
  started = true;
  refetch();
  void listen("extensions-reloaded", refetch);
  void listen("search-invalidated", refetch);
}

const subscribe = (cb: () => void) => {
  ensureStarted();
  listeners.add(cb);
  return () => void listeners.delete(cb);
};

/** Reactive command catalog (empty until the first fetch lands). */
export function useCommands(): CommandDescriptor[] {
  return useSyncExternalStore(subscribe, () => cache);
}

/** Imperative catalog snapshot for module-level consumers (keybinds store). */
export function getCommands(): CommandDescriptor[] {
  ensureStarted();
  return cache;
}

/** Module-level subscription (non-hook counterpart of useCommands). */
export function subscribeCommands(cb: () => void): () => void {
  return subscribe(cb);
}

/** Sync lookup with built-in fallbacks for the two takeover/Tab paths. */
export function commandById(id: string): CommandDescriptor | undefined {
  ensureStarted();
  const hit = cache.find(c => c.id === id);
  if (hit) return hit;
  if (id === FALLBACK_CONTENTS.id) return FALLBACK_CONTENTS;
  if (id === FALLBACK_CLIPBOARD.id) return FALLBACK_CLIPBOARD;
  return undefined;
}
