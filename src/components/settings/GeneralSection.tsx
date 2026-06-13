import { Config } from "../../types";
import Toggle from "./Toggle";
import SectionHeader from "./SectionHeader";
import SettingsGroup from "./SettingsGroup";
import SettingsField from "./SettingsField";
import NumberStepper from "./NumberStepper";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

export default function GeneralSection({ config, onChange }: Props) {
  const setGeneral = (patch: Partial<Config["general"]>) =>
    onChange({ ...config, general: { ...config.general, ...patch } });

  return (
    <div className="settings-section">
      <SectionHeader title="General" desc="How the launcher window behaves." />

      <SettingsGroup>
        <SettingsField name="Max results" desc="Total results shown in the launcher per query.">
          <NumberStepper
            label="Max results"
            value={config.general.max_results}
            min={1}
            max={50}
            onChange={max_results => setGeneral({ max_results })}
          />
        </SettingsField>

        <SettingsField
          name="Layer-shell overlay"
          desc="Wayland only. Draw the launcher as a true overlay above all windows. Restart to apply."
        >
          <Toggle label="Layer-shell overlay" checked={config.general.layer_shell} onChange={layer_shell => setGeneral({ layer_shell })} />
        </SettingsField>
      </SettingsGroup>
    </div>
  );
}
