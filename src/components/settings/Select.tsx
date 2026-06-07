import { useState, useEffect, useRef } from "react";

interface SelectOption {
  label: string;
}

interface Props {
  options: SelectOption[];
  value: string;
  onChange: (label: string) => void;
}

export default function Select({ options, value, onChange }: Props) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div className="settings-select-wrap" ref={wrapRef}>
      <button
        type="button"
        className={`settings-select-btn${open ? " open" : ""}`}
        onClick={() => setOpen(o => !o)}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        <span>{value}</span>
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
          <path d="M2 3.5l3 3 3-3"/>
        </svg>
      </button>
      {open && (
        <div className="settings-select-dropdown" role="listbox">
          {options.map(o => (
            <button
              key={o.label}
              type="button"
              role="option"
              aria-selected={o.label === value}
              className={`settings-select-option${o.label === value ? " selected" : ""}`}
              onClick={() => { onChange(o.label); setOpen(false); }}
            >
              {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
