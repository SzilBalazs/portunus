import { useRef, useState } from "react";
import Badge from "../Badge";
import ShortcutChip from "./ShortcutChip";
import ShortcutRecorder from "./ShortcutRecorder";
import { EDITING_CHORDS } from "../../../keybinds/chord";
import type { ConflictInfo } from "./conflicts";

export interface KeybindRowProps {
  id: string;
  title: string;
  hint?: string;
  /** Effective chords shown on the chip. */
  chords: string[];
  /** Shipped default chords (for the reset tooltip). */
  defaults: string[];
  /** A user override exists (drives the accent dot + reset affordance). */
  modified: boolean;
  /** Structural row: locked, no recorder. */
  fixed?: boolean;
  /** Orphan config entry whose target no longer exists. */
  missing?: boolean;
  conflict?: ConflictInfo;
  recording: boolean;
  onStartRecord: () => void;
  /** Canonical chord, or "" to clear. */
  onCommit: (chord: string) => void;
  onCancelRecord: () => void;
  onReset: () => void;
  /** Missing rows: delete the orphan config entry outright. */
  onDelete?: () => void;
  /** Badge click filters the list to the shared chord. */
  onConflictClick?: (chord: string) => void;
}

export default function KeybindRow(p: KeybindRowProps) {
  const chipRef = useRef<HTMLButtonElement>(null);
  const [flash, setFlash] = useState(0);
  const [note, setNote] = useState<string | null>(null);
  const noteTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const finish = () => {
    // Recording swaps the chip out; give focus back where the user was.
    requestAnimationFrame(() => chipRef.current?.focus());
  };
  const commit = (chord: string) => {
    p.onCommit(chord);
    setFlash(f => f + 1);
    if (chord && EDITING_CHORDS.has(chord)) {
      setNote("Note: this shadows a common text-editing chord");
      if (noteTimer.current) clearTimeout(noteTimer.current);
      noteTimer.current = setTimeout(() => setNote(null), 3000);
    }
    finish();
  };

  const chordLabel = p.chords.join(", ") || "none";
  return (
    <div className={`settings-keybind-row${p.fixed ? " settings-keybind-row--fixed" : ""}${p.missing ? " settings-keybind-row--missing" : ""}`}>
      <div className="settings-keybind-text">
        <div className="settings-keybind-title">
          {p.missing ? <span className="settings-keybind-id">{p.id}</span> : p.title}
        </div>
        {(note ?? p.hint) && (
          <div className={`settings-keybind-hint${note ? " settings-keybind-hint--note" : ""}`}>
            {note ?? p.hint}
          </div>
        )}
        {p.recording && !note && (
          <div className="settings-keybind-hint settings-keybind-hintline">Esc cancel · ⌫ clear</div>
        )}
      </div>
      <div className="settings-keybind-controls">
        {p.conflict && (
          <button
            type="button"
            className="settings-keybind-conflict"
            title={`Also bound to: ${p.conflict.with.join(", ")}`}
            onClick={() => p.onConflictClick?.(p.conflict!.chord)}
          >
            <Badge tone={p.conflict.tone === "error" ? "error" : "neutral"}>{p.conflict.label}</Badge>
          </button>
        )}
        {p.modified && !p.missing && (
          <span
            className="settings-keybind-dot"
            title={`Modified — reset restores ${p.defaults.length ? p.defaults.join(", ") : "no shortcut"}`}
          />
        )}
        {p.modified && !p.missing && !p.recording && (
          <button
            type="button"
            className="settings-keybind-reset"
            title={`Reset to default (${p.defaults.length ? p.defaults.join(", ") : "none"})`}
            aria-label={`Reset shortcut for ${p.title}`}
            onClick={p.onReset}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <polyline points="1 4 1 10 7 10"/><path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10"/>
            </svg>
          </button>
        )}
        {p.missing ? (
          <>
            <ShortcutChip chords={p.chords} fixed ariaLabel={`Orphan binding ${chordLabel}`} />
            <button
              type="button"
              className="settings-keybind-reset settings-keybind-trash"
              title="Remove this binding"
              aria-label={`Remove orphan binding for ${p.id}`}
              onClick={p.onDelete}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>
              </svg>
            </button>
          </>
        ) : p.recording ? (
          <ShortcutRecorder
            allowPlainTab={p.id === "builtin:contents"}
            onCommit={commit}
            onCancel={() => { p.onCancelRecord(); finish(); }}
          />
        ) : (
          <ShortcutChip
            buttonRef={chipRef}
            chords={p.chords}
            fixed={p.fixed}
            flash={flash}
            ariaLabel={
              p.fixed
                ? `${p.title}: ${chordLabel} (fixed)`
                : `Change shortcut for ${p.title}, currently ${chordLabel}`
            }
            onClick={p.fixed ? undefined : p.onStartRecord}
          />
        )}
      </div>
    </div>
  );
}
