import { useState, KeyboardEvent } from "react";

interface Props {
  values: string[];
  onChange: (v: string[]) => void;
  placeholder?: string;
  /** Applied to a committed entry (e.g. strip a leading dot for file types). */
  normalize?: (raw: string) => string;
}

/**
 * Tag-input for a list of short strings: type + Enter/comma/space to add,
 * Backspace on an empty input removes the last, × removes one. The shared
 * implementation behind file-type and ignore-name lists — the only difference
 * between them is `normalize`.
 */
export default function TagEditor({ values, onChange, placeholder = "add…", normalize }: Props) {
  const [draft, setDraft] = useState("");

  const commit = () => {
    const val = (normalize ?? (s => s))(draft.trim());
    if (val && !values.includes(val)) onChange([...values, val]);
    setDraft("");
  };

  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" || e.key === "," || e.key === " ") { e.preventDefault(); commit(); }
    if (e.key === "Backspace" && draft === "" && values.length > 0) {
      onChange(values.slice(0, -1));
    }
  };

  return (
    <div className="settings-ext-editor">
      {values.map(val => (
        <span className="settings-ext-tag" key={val}>
          {val}
          <button className="settings-ext-remove" onClick={() => onChange(values.filter(v => v !== val))} title="Remove">×</button>
        </span>
      ))}
      <input
        className="settings-ext-input"
        value={draft}
        onChange={e => setDraft(e.target.value)}
        onKeyDown={onKey}
        onBlur={commit}
        placeholder={placeholder}
      />
    </div>
  );
}
