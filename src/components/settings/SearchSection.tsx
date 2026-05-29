import { Config } from "../../types";
import NumberField from "./NumberField";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

export default function SearchSection({ config, onChange }: Props) {
  const set = (patch: Partial<Config["search"]>) =>
    onChange({ ...config, search: { ...config.search, ...patch } });

  return (
    <div>
      <div className="settings-section-header">
        <div className="settings-section-name">Search</div>
        <div className="settings-section-desc">Tune fuzzy match quality thresholds and recency scoring.</div>
      </div>

      <NumberField
        label="File score threshold"
        desc="Minimum fuzzy match quality for files and folders (0–255). Higher = stricter matching."
        value={config.search.min_score_file}
        min={0} max={255} step={1}
        onChange={v => set({ min_score_file: v })}
      />
      <NumberField
        label="App score threshold"
        desc="Minimum fuzzy match quality for applications (0–255). Higher = stricter matching."
        value={config.search.min_score_app}
        min={0} max={255} step={1}
        onChange={v => set({ min_score_app: v })}
      />
      <NumberField
        label="Recency weight"
        desc="Maximum bonus added to file scores for recently modified items. Decays linearly to 0 at 1 year old."
        value={config.search.recency_weight}
        min={0} max={500} step={5}
        onChange={v => set({ recency_weight: v })}
      />
    </div>
  );
}
