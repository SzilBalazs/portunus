import { useRef, useState } from "react";
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
  /** Conflict marker click jumps to the Conflicts filter. */
  onConflictClick?: () => void;
}

/** Warning triangle - a hard, same-context conflict. */
const WarnIcon = () => (
  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
    <line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>
  </svg>
);

/** Swap arrows - a soft, cross-context "shared" chord (legal by design). */
const SwapIcon = () => (
  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <polyline points="17 1 21 5 17 9"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/>
    <polyline points="7 23 3 19 7 15"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/>
  </svg>
);

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
  const c = p.conflict;
  // Faint qualifier after the word: which other binding(s) share the chord.
  const conflictNote = c
    ? c.tone === "error"
      ? `with ${c.with[0]}`
      : c.with.length > 1
        ? `· ${c.with.length} actions`
        : `with ${c.with[0]}`
    : "";
  const rowClass = [
    "settings-keybind-row",
    p.fixed && "settings-keybind-row--fixed",
    p.missing && "settings-keybind-row--missing",
    p.modified && !p.missing && "settings-keybind-row--modified",
  ].filter(Boolean).join(" ");
  return (
    <div className={rowClass}>
      <div className="settings-keybind-text">
        <div className="settings-keybind-title">
          {p.missing ? <span className="settings-keybind-id">{p.id}</span> : p.title}
        </div>
        {(note ?? p.hint) && (
          <div className={`settings-keybind-hint${note ? " settings-keybind-hint--note" : ""}`}>
            {note ?? p.hint}
          </div>
        )}
      </div>
      <div className="settings-keybind-controls">
        {c && (
          <button
            type="button"
            className={`settings-keybind-conflict settings-keybind-conflict--${c.tone === "error" ? "error" : "shared"}`}
            title={`Also bound to: ${c.with.join(", ")}`}
            onClick={() => p.onConflictClick?.()}
          >
            <span className="settings-keybind-conflict-icon">{c.tone === "error" ? <WarnIcon /> : <SwapIcon />}</span>
            <span className="settings-keybind-conflict-word">{c.tone === "error" ? "Conflict" : "Shared"}</span>
            {conflictNote && <span className="settings-keybind-conflict-note">{conflictNote}</span>}
          </button>
        )}
        {p.modified && !p.missing && !p.recording && (
          <button
            type="button"
            className="settings-keybind-reset settings-keybind-reset--text"
            title={`Reset to default (${p.defaults.length ? p.defaults.join(", ") : "none"})`}
            aria-label={`Reset shortcut for ${p.title}`}
            onClick={p.onReset}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <polyline points="1 4 1 10 7 10"/><path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10"/>
            </svg>
            Reset
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
