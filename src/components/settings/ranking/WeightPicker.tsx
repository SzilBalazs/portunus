import { weightLabel } from "./categories";

interface Props {
  /** 0–100 in steps of 25, snapped to the five stops. */
  value: number;
  onChange: (v: number) => void;
  /** Accessible name for the group (e.g. the category or extension name). */
  label: string;
}

/** The five discrete weight stops. `Hidden` (0) removes the category from
 *  root search; `Default` (50) is neutral. Values mirror the 0–100 config. */
const STOPS = [0, 25, 50, 75, 100] as const;

/**
 * Inline five-stop weight selector: a segmented control where each cell is one
 * level and the current label reads out beside it. Replaces the abstract 0–100
 * slider — the stops *are* the labels (Hidden · Low · Default · High · Max).
 */
export default function WeightPicker({ value, onChange, label }: Props) {
  const idx = Math.max(0, STOPS.findIndex(s => s >= value));
  const hidden = value === 0;

  const step = (dir: -1 | 1) => {
    const next = STOPS[Math.min(STOPS.length - 1, Math.max(0, idx + dir))];
    if (next !== value) onChange(next);
  };

  return (
    <div className="weight-picker">
      <div
        className="weight-seg"
        role="radiogroup"
        aria-label={`${label} weight`}
        onKeyDown={e => {
          if (e.key === "ArrowLeft" || e.key === "ArrowDown") { e.preventDefault(); step(-1); }
          if (e.key === "ArrowRight" || e.key === "ArrowUp") { e.preventDefault(); step(1); }
        }}
      >
        {STOPS.map((stop, i) => {
          const on = i === idx;
          return (
            <button
              key={stop}
              type="button"
              role="radio"
              aria-checked={on}
              aria-label={weightLabel(stop)}
              tabIndex={on ? 0 : -1}
              className={`weight-cell${on ? " on" : ""}${on && stop === 0 ? " hide" : ""}`}
              onClick={() => onChange(stop)}
            />
          );
        })}
      </div>
      <span className={`weight-readout${hidden ? " hide" : ""}`}>{weightLabel(value)}</span>
    </div>
  );
}
