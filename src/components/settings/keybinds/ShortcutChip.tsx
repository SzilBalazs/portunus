import { EnterIcon } from "../../../icons";
import { chordParts } from "../../../keybinds/chord";

/** Display text for one chord token ("ctrl" stays, letters uppercase). */
const tokenLabel = (t: string) => {
  if (t.length === 1) return t.toUpperCase();
  switch (t) {
    case "tab": return "Tab";
    case "space": return "Space";
    case "escape": return "Esc";
    case "up": return "↑";
    case "down": return "↓";
    case "comma": return ",";
    case "period": return ".";
    case "slash": return "/";
    case "semicolon": return ";";
    case "quote": return "'";
    case "backquote": return "`";
    case "minus": return "-";
    case "equal": return "=";
    case "bracketleft": return "[";
    case "bracketright": return "]";
    case "backslash": return "\\";
    default: return t;
  }
};

/** The kbd tokens of one chord ("ctrl+shift+q" → [ctrl][shift][Q]). */
export function ChordKbds({ chord }: { chord: string }) {
  return (
    <>
      {chordParts(chord).map((t, i) => (
        <kbd key={i}>{t === "enter" ? <EnterIcon /> : tokenLabel(t)}</kbd>
      ))}
    </>
  );
}

interface Props {
  /** Effective chords; empty = unbound ("None"). */
  chords: string[];
  /** Locked structural row - dimmed, not clickable as a recorder. */
  fixed?: boolean;
  /** One-shot accent flash after a commit (key change retriggers). */
  flash?: number;
  onClick?: () => void;
  ariaLabel?: string;
  buttonRef?: React.Ref<HTMLButtonElement>;
}

/** The clickable chord display of a keybind row; clicking starts recording. */
export default function ShortcutChip({ chords, fixed, flash, onClick, ariaLabel, buttonRef }: Props) {
  return (
    <button
      ref={buttonRef}
      type="button"
      className={`settings-keybind-chip${fixed ? " settings-keybind-chip--fixed" : ""}${chords.length === 0 ? " settings-keybind-chip--none" : ""}${flash ? " settings-keybind-chip--flash" : ""}`}
      key={flash}
      onClick={onClick}
      aria-label={ariaLabel}
      title={fixed ? "This key is fixed" : undefined}
    >
      {chords.length === 0
        ? (
          <span className="settings-keybind-add">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
            </svg>
            Add shortcut
          </span>
        )
        : chords.map((c, i) => (
            <span className="settings-keybind-chord" key={i}>
              <ChordKbds chord={c} />
            </span>
          ))}
      {fixed && (
        <svg className="settings-keybind-lock" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>
        </svg>
      )}
    </button>
  );
}
