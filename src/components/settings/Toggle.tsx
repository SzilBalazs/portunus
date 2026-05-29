interface Props {
  checked: boolean;
  onChange: (v: boolean) => void;
  /** Accessible name for screen readers (the visual label sits in a sibling element). */
  label?: string;
}

export default function Toggle({ checked, onChange, label }: Props) {
  return (
    <label className="toggle-wrap">
      <input
        type="checkbox"
        className="toggle-input"
        checked={checked}
        aria-label={label}
        onChange={e => onChange(e.target.checked)}
      />
      <span className="toggle-track"><span className="toggle-thumb" /></span>
    </label>
  );
}
