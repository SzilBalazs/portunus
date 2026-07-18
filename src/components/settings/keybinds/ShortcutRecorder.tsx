import { useEffect, useRef, useState } from "react";
import { canonicalChord, eventToChord, validateChord } from "../../../keybinds/chord";

interface Props {
  /** Plain Tab is allowed only for builtin:contents (its shipped default). */
  allowPlainTab?: boolean;
  /** Commit a canonical chord; "" clears the binding. */
  onCommit: (chord: string) => void;
  onCancel: () => void;
}

const MOD_KEYS = new Set(["Control", "Alt", "Shift", "Meta"]);

// Modifier state straight off the event - correct on both keydown (the
// pressed modifier's flag is already set) and keyup (already cleared).
const heldMods = (e: KeyboardEvent): string[] => {
  const out: string[] = [];
  if (e.ctrlKey) out.push("ctrl");
  if (e.altKey) out.push("alt");
  if (e.shiftKey) out.push("shift");
  return out;
};

/**
 * In-place chord capture, swapped in for the ShortcutChip while recording.
 * Capture-phase window listeners with stopImmediatePropagation starve the
 * Settings Escape-hides-window handler and WebKitGTK's own Ctrl+P/Ctrl+H
 * commands (same escape hatch as Modal.tsx). Escape cancels, bare
 * Backspace/Delete clears, any valid chord commits on keydown; invalid or
 * reserved chords shake and keep recording.
 */
export default function ShortcutRecorder({ allowPlainTab, onCommit, onCancel }: Props) {
  const [held, setHeld] = useState<string[]>([]);
  const [invalid, setInvalid] = useState<{ reason: string; nonce: number } | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const invalidTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Callbacks live in refs so the capture listeners register exactly once.
  const commitRef = useRef(onCommit);
  const cancelRef = useRef(onCancel);
  commitRef.current = onCommit;
  cancelRef.current = onCancel;
  const allowTabRef = useRef(allowPlainTab);
  allowTabRef.current = allowPlainTab;

  useEffect(() => {
    const flashInvalid = (reason: string) => {
      setInvalid(prev => ({ reason, nonce: (prev?.nonce ?? 0) + 1 }));
      // Imperative shake so consecutive misses retrigger reliably.
      if (ref.current && !window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
        ref.current.animate(
          [
            { transform: "translateX(0)" },
            { transform: "translateX(-4px)" },
            { transform: "translateX(4px)" },
            { transform: "translateX(-2px)" },
            { transform: "translateX(0)" },
          ],
          { duration: 300, easing: "ease-out" },
        );
      }
      if (invalidTimer.current) clearTimeout(invalidTimer.current);
      invalidTimer.current = setTimeout(() => setInvalid(null), 1500);
    };
    const onDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopImmediatePropagation();
      if (e.repeat) return;
      if (e.key === "Escape") { cancelRef.current(); return; }
      if ((e.key === "Backspace" || e.key === "Delete")
          && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        commitRef.current("");
        return;
      }
      if (MOD_KEYS.has(e.key)) { setHeld(heldMods(e)); return; }
      if (e.metaKey) { flashInvalid("Super is reserved for the system"); return; }
      const chord = eventToChord(e);
      if (!chord) { flashInvalid("Unrecognized key"); return; }
      const v = validateChord(chord, { allowPlainTab: allowTabRef.current });
      if (!v.ok) { flashInvalid(v.reason); return; }
      commitRef.current(canonicalChord(chord) ?? chord);
    };
    const onUp = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopImmediatePropagation();
      setHeld(heldMods(e));
    };
    const onMouseDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) cancelRef.current();
    };
    const onBlur = () => cancelRef.current();
    window.addEventListener("keydown", onDown, true);
    window.addEventListener("keyup", onUp, true);
    window.addEventListener("mousedown", onMouseDown, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onDown, true);
      window.removeEventListener("keyup", onUp, true);
      window.removeEventListener("mousedown", onMouseDown, true);
      window.removeEventListener("blur", onBlur);
      if (invalidTimer.current) clearTimeout(invalidTimer.current);
    };
  }, []);

  return (
    <div
      ref={ref}
      className={`settings-keybind-recorder${invalid ? " invalid" : ""}`}
      role="status"
    >
      <span className="settings-keybind-recorder-label" aria-live="polite">
        {invalid ? (
          <span className="settings-keybind-recorder-reason">{invalid.reason}</span>
        ) : held.length > 0 ? (
          <>
            {held.map(m => <kbd key={m}>{m}</kbd>)}
            <span className="settings-keybind-recorder-ellipsis">…</span>
          </>
        ) : (
          <span className="settings-keybind-recorder-prompt">Press keys…</span>
        )}
      </span>
    </div>
  );
}
