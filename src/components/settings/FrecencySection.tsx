import { Config } from "../../types";
import Toggle from "./Toggle";
import NumberField from "./NumberField";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

export default function FrecencySection({ config, onChange }: Props) {
  const set = (patch: Partial<Config["frecency"]>) =>
    onChange({ ...config, frecency: { ...config.frecency, ...patch } });

  const fr = config.frecency;

  return (
    <div>
      <div className="settings-section-header">
        <div className="settings-section-name">Frecency</div>
        <div className="settings-section-desc">Boosts items you launch frequently to the top of results.</div>
      </div>

      <div className="settings-field">
        <div className="settings-field-label">
          <div className="settings-field-name">Enable frecency</div>
          <div className="settings-field-desc">Track launch history to surface frequently used apps and files. Stored in SQLite at $XDG_DATA_HOME/portunus/frecency.db</div>
        </div>
        <div className="settings-field-control">
          <Toggle label="Enable frecency" checked={fr.enabled} onChange={v => set({ enabled: v })} />
        </div>
      </div>

      <NumberField
        label="Half-life (days)"
        desc="Frecency score halves every N days of non-use. Shorter = fades faster; longer = longer memory."
        value={fr.half_life_days}
        min={1} max={365} step={1}
        onChange={v => set({ half_life_days: v })}
      />

      <NumberField
        label="Score weight"
        desc="Multiplier applied to the frecency bonus on top of the base category score. Higher = frecency has more influence."
        value={fr.weight}
        min={0} max={50000} step={500}
        width={72}
        onChange={v => set({ weight: v })}
      />
    </div>
  );
}
