export interface FilterChip {
  key: string;
  label: string;
  count: number;
}

interface Props {
  chips: FilterChip[];
  value: string;
  onChange: (key: string) => void;
}

/**
 * A row of pill filter chips, each carrying a count. The active chip reads in
 * the accent; the rest are quiet outlines. A single-select segmented filter.
 */
export default function FilterChips({ chips, value, onChange }: Props) {
  return (
    <div className="settings-filter-chips" role="tablist">
      {chips.map(c => {
        const active = c.key === value;
        return (
          <button
            key={c.key}
            type="button"
            role="tab"
            aria-selected={active}
            className={`settings-filter-chip${active ? " settings-filter-chip--active" : ""}`}
            onClick={() => onChange(c.key)}
          >
            {c.label}
            <span className="settings-filter-chip-count">{c.count}</span>
          </button>
        );
      })}
    </div>
  );
}
