import { Config } from "../../types";
import Toggle from "./Toggle";
import NumberField from "./NumberField";
import Select from "./Select";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

const STRICTNESS_OPTIONS = [
  { label: "Loose",    value: 0.03 },
  { label: "Balanced", value: 0.06 },
  { label: "Strict",   value: 0.12 },
] as const;

function strictnessLabel(v: number): string {
  const match = STRICTNESS_OPTIONS.find(o => Math.abs(o.value - v) < 0.001);
  return match?.label ?? `Custom (${v.toFixed(2)})`;
}

export default function RankingSection({ config, onChange }: Props) {
  const setSearch = (patch: Partial<Config["search"]>) =>
    onChange({ ...config, search: { ...config.search, ...patch } });
  const setFrecency = (patch: Partial<Config["frecency"]>) =>
    onChange({ ...config, frecency: { ...config.frecency, ...patch } });

  const fr = config.frecency;

  return (
    <div>
      <div className="settings-section-header">
        <div className="settings-section-name">Ranking</div>
        <div className="settings-section-desc">Control how results are ordered: history, match quality, and strictness.</div>
      </div>

      <NumberField
        label="History boost"
        desc="How strongly launch history promotes items you use often (0 = off, 100 = max). At 50+, frequently used files can rank above apps."
        value={config.search.history_weight}
        min={0} max={100} step={5}
        onChange={v => setSearch({ history_weight: v })}
      />

      <div className="settings-field">
        <div className="settings-field-label">
          <div className="settings-field-name">Match strictness</div>
          <div className="settings-field-desc">How closely the query must match. Loose shows more results; Strict filters aggressively.</div>
        </div>
        <div className="settings-field-control">
          <Select
            options={STRICTNESS_OPTIONS.map(o => ({ label: o.label }))}
            value={strictnessLabel(config.search.min_quality)}
            onChange={label => {
              const opt = STRICTNESS_OPTIONS.find(o => o.label === label);
              if (opt) setSearch({ min_quality: opt.value });
            }}
          />
        </div>
      </div>

      <div className="settings-section-header" style={{ marginTop: 24 }}>
        <div className="settings-section-name">History</div>
        <div className="settings-section-desc">Tracks how often you launch items so they surface faster over time.</div>
      </div>

      <div className="settings-field">
        <div className="settings-field-label">
          <div className="settings-field-name">Enable history</div>
          <div className="settings-field-desc">Track launch history to surface frequently used apps and files. Stored in SQLite at $XDG_DATA_HOME/portunus/frecency.db</div>
        </div>
        <div className="settings-field-control">
          <Toggle label="Enable history" checked={fr.enabled} onChange={v => setFrecency({ enabled: v })} />
        </div>
      </div>

      <NumberField
        label="Half-life (days)"
        desc="History score halves every N days of non-use. Shorter = fades faster; longer = longer memory."
        value={fr.half_life_days}
        min={1} max={365} step={1}
        onChange={v => setFrecency({ half_life_days: v })}
      />
    </div>
  );
}
