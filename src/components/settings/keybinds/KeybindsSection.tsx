import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Config, ChordList, KeybindsConfig, SeenExtAction } from "../../../types";
import { useTauriListener } from "../../../hooks/useTauriListener";
import SectionHeader from "../SectionHeader";
import SettingsCard from "../SettingsCard";
import FilterChips from "../FilterChips";
import TextInput from "../TextInput";
import Modal from "../Modal";
import KeybindRow from "./KeybindRow";
import { buildConflicts, contextOf, type ConflictTarget } from "./conflicts";
import { BUILTIN_BINDINGS } from "../../../keybinds/builtins";
import { canonicalChord, chordParts } from "../../../keybinds/chord";
import { useCommands } from "../../../commands/store";
import { listProviderActionTargets } from "../../../providers/registry";
// The registry is empty until the provider modules register themselves - the
// launcher window imports this barrel from App.tsx, the settings window here.
import "../../../providers";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

type Table = keyof KeybindsConfig;
type ChipKey = "all" | "modified" | "conflicts" | "unbound";

interface Target {
  /** Full target id ("builtin:pin", "cmd:settings", "ext:ytm:queue_last"). */
  id: string;
  /** Config table + key within it (builtin uses the bare name). */
  table: Table;
  key: string;
  title: string;
  hint?: string;
  /** Shipped default chords. */
  defaults: string[];
  fixed?: boolean;
  /** Orphan config entry - target no longer exists (extension uninstalled). */
  missing?: boolean;
}

interface Group {
  key: string;
  label: string;
  targets: Target[];
}

const EMPTY_KEYBINDS: KeybindsConfig = { builtin: {}, commands: {}, actions: {} };

const chordsOf = (list: ChordList | undefined): string[] | undefined => {
  if (list === undefined) return undefined;
  const arr = typeof list === "string" ? (list === "" ? [] : [list]) : list;
  return arr.map(c => canonicalChord(c) ?? c).filter(c => c !== "");
};

export default function KeybindsSection({ config, onChange }: Props) {
  const kb = config.keybinds ?? EMPTY_KEYBINDS;
  const commands = useCommands();
  const [filter, setFilter] = useState("");
  const [chip, setChip] = useState<ChipKey>("all");
  const [recordingId, setRecordingId] = useState<string | null>(null);
  const [resetGroup, setResetGroup] = useState<Group | null>(null);

  // Seen extension result actions (bounded backend catalog; an extension's
  // actions appear once its results have been shown at least once).
  const [seen, setSeen] = useState<Record<string, SeenExtAction[]>>({});
  const fetchSeen = useCallback(() => {
    invoke<Record<string, SeenExtAction[]>>("list_extension_actions")
      .then(setSeen)
      .catch(() => {});
  }, []);
  useEffect(fetchSeen, [fetchSeen]);
  useTauriListener("extensions-reloaded", fetchSeen, [fetchSeen]);

  const groups = useMemo<Group[]>(() => {
    const out: Group[] = [];
    const known = new Set<string>();
    const push = (g: Group) => { g.targets.forEach(t => known.add(`${t.table}:${t.key}`)); out.push(g); };

    push({
      key: "launcher",
      label: "Launcher",
      targets: BUILTIN_BINDINGS.map(b => ({
        id: b.id,
        table: "builtin" as const,
        key: b.id.slice("builtin:".length),
        title: b.title,
        hint: b.hint,
        defaults: b.defaults,
        fixed: b.fixed,
      })),
    });

    push({
      key: "result-actions",
      label: "Result actions",
      targets: listProviderActionTargets().map(a => ({
        id: a.id,
        table: "actions" as const,
        key: a.id,
        title: a.title,
        hint: a.hint,
        defaults: a.defaultChord ? [a.defaultChord] : [],
      })),
    });

    push({
      key: "commands",
      label: "Commands",
      targets: commands.map(c => ({
        id: c.id,
        table: "commands" as const,
        key: c.id,
        title: c.title,
        hint: c.subtitle ?? (c.source.type === "extension" ? c.source.name : undefined),
        defaults: c.default_shortcut ? [canonicalChord(c.default_shortcut) ?? c.default_shortcut] : [],
      })),
    });

    for (const [name, actions] of Object.entries(seen).sort(([a], [b]) => a.localeCompare(b))) {
      if (actions.length === 0) continue;
      push({
        key: `ext-${name}`,
        label: name,
        targets: actions.map(a => ({
          id: `ext:${name}:${a.id}`,
          table: "actions" as const,
          key: `ext:${name}:${a.id}`,
          title: a.label,
          hint: a.hint,
          defaults: a.shortcut ? [canonicalChord(a.shortcut) ?? a.shortcut] : [],
        })),
      });
    }

    // Orphans: config entries whose target vanished (uninstalled extension,
    // renamed action). Never silently pruned - the extension may come back.
    const orphans: Target[] = [];
    for (const table of ["builtin", "commands", "actions"] as Table[]) {
      for (const key of Object.keys(kb[table] ?? {})) {
        if (!known.has(`${table}:${key}`)) {
          orphans.push({ id: key, table, key, title: key, defaults: [], missing: true });
        }
      }
    }
    if (orphans.length > 0) {
      out.push({ key: "missing", label: "Not installed", targets: orphans });
    }
    return out;
  }, [commands, seen, kb]);

  const effective = useCallback(
    (t: Target): string[] => chordsOf(kb[t.table]?.[t.key]) ?? t.defaults,
    [kb],
  );
  const isModified = useCallback(
    (t: Target): boolean => kb[t.table]?.[t.key] !== undefined,
    [kb],
  );

  const conflicts = useMemo(() => {
    const flat: ConflictTarget[] = groups.flatMap(g =>
      g.targets
        .filter(t => !t.fixed && !t.missing)
        .map(t => ({
          id: t.id,
          title: t.title,
          chords: effective(t),
          modified: isModified(t),
          context: contextOf(t.id, t.table),
        })),
    );
    return buildConflicts(flat);
  }, [groups, effective, isModified]);

  // Write path: value undefined deletes the override (back to default).
  const set = (t: Target, value: string | undefined) => {
    const table = { ...(kb[t.table] ?? {}) };
    if (value === undefined) delete table[t.key];
    else table[t.key] = value;
    onChange({ ...config, keybinds: { ...kb, [t.table]: table } });
  };

  const resetAll = (g: Group) => {
    const next = { builtin: { ...kb.builtin }, commands: { ...kb.commands }, actions: { ...kb.actions } };
    for (const t of g.targets) delete next[t.table][t.key];
    onChange({ ...config, keybinds: next });
    setResetGroup(null);
  };

  const isUnbound = useCallback(
    (t: Target): boolean => !t.fixed && !t.missing && effective(t).length === 0,
    [effective],
  );

  // Chip counts span every target, independent of the search needle.
  const counts = useMemo(() => {
    const all = groups.flatMap(g => g.targets);
    return {
      all: all.length,
      modified: all.filter(isModified).length,
      conflicts: all.filter(t => conflicts.has(t.id)).length,
      unbound: all.filter(isUnbound).length,
    };
  }, [groups, isModified, conflicts, isUnbound]);

  const chipMatches = (t: Target): boolean => {
    switch (chip) {
      case "modified": return isModified(t);
      case "conflicts": return conflicts.has(t.id);
      case "unbound": return isUnbound(t);
      default: return true;
    }
  };

  const needle = filter.trim().toLowerCase();
  const matches = (g: Group, t: Target): boolean => {
    if (!needle) return true;
    const chords = effective(t);
    const hay = [
      t.title,
      t.hint ?? "",
      g.label,
      ...chords,
      ...chords.map(c => chordParts(c).join(" ")),
    ].join("\n").toLowerCase();
    return hay.includes(needle);
  };

  const visible = groups
    .map(g => ({ ...g, targets: g.targets.filter(t => chipMatches(t) && matches(g, t)) }))
    .filter(g => g.targets.length > 0);

  const chips = [
    { key: "all", label: "All", count: counts.all },
    { key: "modified", label: "Modified", count: counts.modified },
    { key: "conflicts", label: "Conflicts", count: counts.conflicts },
    { key: "unbound", label: "Unbound", count: counts.unbound },
  ];

  // A recorder on a row that just got filtered out must not keep eating keys.
  useEffect(() => {
    if (!recordingId) return;
    if (!visible.some(g => g.targets.some(t => t.id === recordingId))) setRecordingId(null);
  }, [recordingId, visible]);

  return (
    <>
      <SectionHeader
        title="Keybinds"
        desc="Remap launcher chords, commands, and extension actions. Click a binding and press the new keys."
      />

      <div className="settings-keybind-search">
        <TextInput
          value={filter}
          onChange={setFilter}
          placeholder="Search bindings…"
          label="Search bindings"
        />
      </div>

      <FilterChips chips={chips} value={chip} onChange={k => setChip(k as ChipKey)} />

      {visible.length === 0 && (
        <div className="settings-pin-empty">
          {needle ? <>No bindings match “{filter.trim()}”.</> : "No bindings in this filter."}
        </div>
      )}

      {visible.map(g => {
        const modifiedCount = g.key === "missing" ? 0 : g.targets.filter(isModified).length;
        return (
          <SettingsCard
            key={g.key}
            label={g.label}
            sub={g.key.startsWith("ext-") ? "Extension" : undefined}
            count={g.targets.length}
            action={modifiedCount > 0 ? (
              <button className="settings-keybind-resetall" onClick={() => setResetGroup(g)}>
                Reset all
              </button>
            ) : undefined}
          >
            <div className="settings-keybind-list">
              {g.targets.map(t => (
                <KeybindRow
                  key={t.id}
                  id={t.id}
                  title={t.title}
                  hint={t.hint}
                  chords={effective(t)}
                  defaults={t.defaults}
                  modified={!t.missing && isModified(t)}
                  fixed={t.fixed}
                  missing={t.missing}
                  conflict={conflicts.get(t.id)}
                  recording={recordingId === t.id}
                  onStartRecord={() => setRecordingId(t.id)}
                  onCommit={chord => { set(t, chord); setRecordingId(null); }}
                  onCancelRecord={() => setRecordingId(null)}
                  onReset={() => set(t, undefined)}
                  onDelete={() => set(t, undefined)}
                  onConflictClick={() => setChip("conflicts")}
                />
              ))}
            </div>
          </SettingsCard>
        );
      })}

      {resetGroup && (
        <Modal
          title={`Reset ${resetGroup.label} keybinds`}
          onClose={() => setResetGroup(null)}
          footer={
            <>
              <button className="settings-btn-secondary" onClick={() => setResetGroup(null)}>Cancel</button>
              <button className="settings-btn-danger" onClick={() => resetAll(resetGroup)}>Reset all</button>
            </>
          }
        >
          Restore every binding in “{resetGroup.label}” to its default? Your custom chords in this group will be removed.
        </Modal>
      )}
    </>
  );
}
