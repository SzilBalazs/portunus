// Pure conflict analysis for the Keybinds section. Two bindings on the same
// chord conflict hard ("error") only when they can fire in the same context:
// builtins and command bindings are both launcher-global, and two actions of
// the same extension apply to the same selected result. Cross-context reuse
// (an extension action vs a global chord, or actions of different result
// kinds) is legal by design - the runtime layers them - so it is only
// surfaced as an informational "shared" badge, and only once a user override
// is involved: the shipped defaults deliberately share chords (the Ctrl+C
// copy family) and must not ship with warning badges.

export interface ConflictInfo {
  tone: "error" | "neutral";
  label: "conflict" | "shared";
  /** Titles of the other targets bound to the same chord. */
  with: string[];
  /** The shared chord (badge click filters the list to it). */
  chord: string;
}

export interface ConflictTarget {
  id: string;
  title: string;
  /** Effective chords (empty = unbound). */
  chords: string[];
  /** A user override exists for this target. */
  modified: boolean;
  /** "global" for builtins/commands, "ext:<name>" for extension actions,
   *  "provider:<prefix>" for static provider actions. */
  context: string;
}

/** Context key for a target id. */
export function contextOf(id: string, table: "builtin" | "commands" | "actions"): string {
  if (table !== "actions") return "global";
  const m = /^ext:([^:]+):/.exec(id);
  if (m) return `ext:${m[1]}`;
  return `provider:${id.split(":")[0]}`;
}

/** Map of target id → conflict badge to show (absent = clean). */
export function buildConflicts(targets: ConflictTarget[]): Map<string, ConflictInfo> {
  const byChord = new Map<string, ConflictTarget[]>();
  for (const t of targets) {
    for (const c of t.chords) {
      const list = byChord.get(c);
      if (list) list.push(t);
      else byChord.set(c, [t]);
    }
  }
  const out = new Map<string, ConflictInfo>();
  for (const [chord, list] of byChord) {
    if (list.length < 2) continue;
    for (const t of list) {
      const others = list.filter(o => o !== t);
      const hard = others.some(o => o.context === t.context);
      const anyModified = t.modified || others.some(o => o.modified);
      if (!hard && !anyModified) continue; // shipped cross-context sharing is by design
      const existing = out.get(t.id);
      if (existing?.tone === "error" && !hard) continue; // keep the worst badge
      out.set(t.id, {
        tone: hard ? "error" : "neutral",
        label: hard ? "conflict" : "shared",
        with: others.map(o => o.title),
        chord,
      });
    }
  }
  return out;
}
