interface Props {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  /** Accessible name. */
  label?: string;
  /** Render in monospace (urls, hashes, keys). */
  mono?: boolean;
  /** Fixed width in px; defaults to filling the control slot. */
  width?: number;
  autoFocus?: boolean;
  onEnter?: () => void;
}

/** Single-line text input control - compose inside `SettingsField` or dialogs. */
export default function TextInput({ value, onChange, placeholder, label, mono, width, autoFocus, onEnter }: Props) {
  return (
    <input
      type="text"
      className={`settings-text-input${mono ? " mono" : ""}`}
      style={width ? { width } : undefined}
      value={value}
      placeholder={placeholder}
      aria-label={label}
      autoFocus={autoFocus}
      spellCheck={false}
      onChange={e => onChange(e.target.value)}
      onKeyDown={e => { if (e.key === "Enter") onEnter?.(); }}
    />
  );
}
