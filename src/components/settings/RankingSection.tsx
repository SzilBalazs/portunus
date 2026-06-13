import { Config } from "../../types";
import Toggle from "./Toggle";
import Select from "./Select";
import SectionHeader from "./SectionHeader";
import SettingsGroup from "./SettingsGroup";
import SettingsField from "./SettingsField";
import Slider from "./Slider";

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
  return STRICTNESS_OPTIONS.find(o => Math.abs(o.value - v) < 0.001)?.label ?? `Custom (${v.toFixed(2)})`;
}

export default function RankingSection({ config, onChange }: Props) {
  const setSearch = (patch: Partial<Config["search"]>) =>
    onChange({ ...config, search: { ...config.search, ...patch } });
  const setFrecency = (patch: Partial<Config["frecency"]>) =>
    onChange({ ...config, frecency: { ...config.frecency, ...patch } });

  const fr = config.frecency;

  return (
    <div className="settings-section">
      <SectionHeader title="Ranking" desc="Control how results are ordered: match quality and launch history." />

      <SettingsGroup title="Match quality">
        <SettingsField
          name="Match strictness"
          desc="How closely the query must match. Loose shows more results; Strict filters aggressively."
        >
          <Select
            options={STRICTNESS_OPTIONS.map(o => ({ label: o.label }))}
            value={strictnessLabel(config.search.min_quality)}
            onChange={label => {
              const opt = STRICTNESS_OPTIONS.find(o => o.label === label);
              if (opt) setSearch({ min_quality: opt.value });
            }}
          />
        </SettingsField>
      </SettingsGroup>

      <SettingsGroup
        title="Launch history"
        desc="Tracks how often you launch items so they surface faster over time."
      >
        <SettingsField
          name="Track launch history"
          desc="Remember frequently used apps and files and promote them in results."
        >
          <Toggle label="Track launch history" checked={fr.enabled} onChange={v => setFrecency({ enabled: v })} />
        </SettingsField>

        <SettingsField
          name="History boost"
          desc="How strongly history promotes items you use often. At higher values, frequently used files can rank above apps."
        >
          <Slider
            label="History boost"
            value={config.search.history_weight}
            min={0} max={100} step={5}
            format={v => v === 0 ? "Off" : `${v}`}
            onChange={v => setSearch({ history_weight: v })}
          />
        </SettingsField>

        <SettingsField
          name="Half-life"
          desc="History score halves after this many days of non-use. Shorter fades faster; longer remembers longer."
        >
          <Slider
            label="Half-life (days)"
            value={fr.half_life_days}
            min={1} max={365} step={1}
            format={v => `${Math.round(v)} d`}
            onChange={v => setFrecency({ half_life_days: v })}
          />
        </SettingsField>
      </SettingsGroup>
    </div>
  );
}
