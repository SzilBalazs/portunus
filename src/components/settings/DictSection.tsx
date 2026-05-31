import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Config, DepStatus } from "../../types";
import Toggle from "./Toggle";
import NumberField from "./NumberField";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

export default function DictSection({ config, onChange }: Props) {
  const set = (patch: Partial<Config["dict"]>) =>
    onChange({ ...config, dict: { ...config.dict, ...patch } });

  const [deps, setDeps] = useState<DepStatus[] | null>(null);
  useEffect(() => {
    invoke<DepStatus[]>("check_dependencies").then(setDeps).catch(() => setDeps([]));
  }, []);

  const dictDep = deps?.find(d => d.id === "dict");
  const missing = config.dict.enabled && dictDep && !dictDep.available;

  type BoolKey = "enabled" | "fill_sparse" | "correct_misspellings" | "copy_definition";
  const toggleField = (name: string, desc: string, key: BoolKey) => (
    <div className="settings-field">
      <div className="settings-field-label">
        <div className="settings-field-name">{name}</div>
        <div className="settings-field-desc">{desc}</div>
      </div>
      <div className="settings-field-control">
        <Toggle label={name} checked={config.dict[key]} onChange={v => set({ [key]: v })} />
      </div>
    </div>
  );

  return (
    <div>
      <div className="settings-section-header">
        <div className="settings-section-name">Dictionary</div>
        <div className="settings-section-desc">
          Word definitions via dict. Explicit <code>define</code> and <code>dict</code> lookups,
          plus optional sparse-result fill for plain words.
        </div>
      </div>

      {missing && (
        <div className="settings-dep-inline-warn">
          ⚠ Enabled but <code>{dictDep!.label}</code> is missing — install <code>{dictDep!.install_hint}</code>
        </div>
      )}

      {toggleField("Enabled", "Master switch for dictionary lookups.", "enabled")}
      {toggleField(
        "Fill sparse results",
        "When few other results match a plain word, add dictionary entries for it.",
        "fill_sparse",
      )}
      {toggleField(
        "Correct misspellings",
        "Allow edit-distance (typo) matches when filling. Off = exact word only.",
        "correct_misspellings",
      )}
      {toggleField(
        "Copy definition on Ctrl+C",
        "On = copy the first definition. Off = copy the word itself.",
        "copy_definition",
      )}

      <NumberField
        label="Fill threshold"
        desc="Only fill when fewer than this many non-dictionary results exist."
        value={config.dict.fill_threshold}
        min={0} max={20} step={1}
        onChange={v => set({ fill_threshold: v })}
      />
      <NumberField
        label="Fill max"
        desc="Maximum dictionary rows added when filling."
        value={config.dict.fill_max}
        min={0} max={20} step={1}
        onChange={v => set({ fill_max: v })}
      />
    </div>
  );
}
