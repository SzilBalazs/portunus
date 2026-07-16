import { CSSProperties } from "react";

export interface TickStop {
  /** Config value this stop writes. */
  value: number;
  /** Label shown in the readout when this stop is active. */
  label: string;
}

interface Props {
  /** Discrete stops, low to high. The slider snaps between them. */
  stops: TickStop[];
  /** Current config value; the nearest stop drives the thumb. */
  value: number;
  onChange: (v: number) => void;
  /** Accessible name. */
  label: string;
}

/** Index of the stop whose value is closest to `value`. */
function nearest(stops: TickStop[], value: number): number {
  let best = 0;
  let bestDist = Infinity;
  for (let i = 0; i < stops.length; i++) {
    const d = Math.abs(stops[i].value - value);
    if (d < bestDist) { bestDist = d; best = i; }
  }
  return best;
}

/**
 * A range slider that snaps to a handful of named stops, with a notch under
 * each and the active stop's label as the readout. The input rides the stop
 * *index* (step 1) so each field can map a level to its own magnitude while the
 * ticks stay evenly spaced. Honest discrete levels, no vague in-between words.
 */
export default function TickSlider({ stops, value, onChange, label }: Props) {
  const last = stops.length - 1;
  const idx = nearest(stops, value);
  const pct = last > 0 ? (idx / last) * 100 : 0;

  return (
    <div className="tick-slider">
      <div className="tick-slider-track">
        <input
          type="range"
          className="tick-slider-input"
          min={0}
          max={last}
          step={1}
          value={idx}
          aria-label={label}
          aria-valuetext={stops[idx].label}
          style={{ "--pct": `${pct}%` } as CSSProperties}
          onChange={e => {
            const i = parseInt(e.target.value, 10);
            if (i !== idx) onChange(stops[i].value);
          }}
        />
        <div className="tick-slider-notches" aria-hidden="true">
          {stops.map((s, i) => {
            const hidden = i === 0 || i === last || i === idx;
            return (
              <span key={s.value} className={`tick-slider-notch${hidden ? " hidden" : ""}`} />
            );
          })}
        </div>
      </div>
      <span className="tick-slider-value">{stops[idx].label}</span>
    </div>
  );
}
