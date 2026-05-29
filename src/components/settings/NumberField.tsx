interface Props {
  label: string;
  desc: string;
  value: number;
  min?: number;
  max?: number;
  step?: number;
  /** Optional unit shown after the input (e.g. "MB"). */
  suffix?: string;
  /** Optional fixed input width in px (for wide values). */
  width?: number;
  onChange: (v: number) => void;
}

export default function NumberField({
  label, desc, value, min, max, step, suffix, width, onChange,
}: Props) {
  const dec = () => onChange(Math.max(min ?? 0, value - (step ?? 1)));
  const inc = () => onChange(Math.min(max ?? Infinity, value + (step ?? 1)));
  return (
    <div className="settings-field">
      <div className="settings-field-label">
        <div className="settings-field-name">{label}</div>
        <div className="settings-field-desc">{desc}</div>
      </div>
      <div className="settings-field-control">
        <div className="settings-number-wrap">
          <button className="settings-number-btn" onClick={dec} aria-label={`Decrease ${label}`}>−</button>
          <input
            type="number"
            className="settings-number-input"
            style={width ? { width } : undefined}
            value={value}
            min={min}
            max={max}
            step={step}
            aria-label={label}
            onChange={e => {
              const v = parseFloat(e.target.value);
              // Reject NaN and values below the floor so typing can't push a field
              // into an invalid range (e.g. a zero half-life).
              if (!isNaN(v) && (min === undefined || v >= min)) onChange(v);
            }}
          />
          <button className="settings-number-btn" onClick={inc} aria-label={`Increase ${label}`}>+</button>
        </div>
        {suffix && <span style={{ marginLeft: 8, fontSize: 11, color: "var(--fg-mute)" }}>{suffix}</span>}
      </div>
    </div>
  );
}
