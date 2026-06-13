import { Config } from "../../types";
import Toggle from "./Toggle";
import SectionHeader from "./SectionHeader";
import SettingsGroup from "./SettingsGroup";
import SettingsField from "./SettingsField";
import NumberStepper from "./NumberStepper";
import { useDep } from "./DepsContext";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

export default function DictSection({ config, onChange }: Props) {
  const set = (patch: Partial<Config["dict"]>) =>
    onChange({ ...config, dict: { ...config.dict, ...patch } });

  const dictDep = useDep("dict");
  const missing = config.dict.enabled && dictDep && !dictDep.available;
  const disabled = !config.dict.enabled;

  return (
    <div className="settings-section">
      <SectionHeader
        title="Dictionary"
        desc={<>Look up word definitions with <code>define</code> or <code>dict</code>, and optionally show them automatically when few other results match.</>}
        master={{ checked: config.dict.enabled, onChange: v => set({ enabled: v }), label: "Enable dictionary" }}
        warn={missing && (
          <div className="settings-dep-inline-warn">
            ⚠ Enabled but <code>{dictDep!.label}</code> is missing. Install <code>{dictDep!.install_hint}</code>
          </div>
        )}
      />

      <div className={disabled ? "settings-disabled" : undefined} aria-hidden={disabled}>
        <SettingsGroup title="Automatic definitions">
          <SettingsField
            name="Show definitions when little else matches"
            desc="When few other results match a plain word, add its dictionary definition."
          >
            <Toggle label="Show definitions when little else matches" checked={config.dict.fill_sparse} onChange={v => set({ fill_sparse: v })} />
          </SettingsField>

          <SettingsField
            name="Correct misspellings"
            desc="Match words even with small typos. Off = exact spelling only."
          >
            <Toggle label="Correct misspellings" checked={config.dict.correct_misspellings} onChange={v => set({ correct_misspellings: v })} />
          </SettingsField>

          <SettingsField
            name="Result threshold"
            desc="Only add definitions when fewer than this many other results match."
          >
            <NumberStepper label="Result threshold" value={config.dict.fill_threshold} min={0} max={20} onChange={v => set({ fill_threshold: v })} />
          </SettingsField>

          <SettingsField name="Max definitions" desc="Most definition rows to add at once.">
            <NumberStepper label="Max definitions" value={config.dict.fill_max} min={0} max={20} onChange={v => set({ fill_max: v })} />
          </SettingsField>
        </SettingsGroup>

        <SettingsGroup title="Behaviour">
          <SettingsField
            name="Copy definition on Ctrl+C"
            desc="On = copy the first definition. Off = copy the word itself."
          >
            <Toggle label="Copy definition on Ctrl+C" checked={config.dict.copy_definition} onChange={v => set({ copy_definition: v })} />
          </SettingsField>
        </SettingsGroup>
      </div>
    </div>
  );
}
