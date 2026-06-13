import { useState } from "react";
import { Config } from "../../types";
import Toggle from "./Toggle";
import SectionHeader from "./SectionHeader";
import SettingsGroup from "./SettingsGroup";
import SettingsField from "./SettingsField";
import { useDeps } from "./DepsContext";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

/**
 * Toggleable search sources, each bound to its real config path (so the toggle
 * stays in sync with the same source's per-tab header master switch).
 * `dep` names an optional tool the source needs to function.
 * Content search lives in its own tab (staged reindex), so it's not duplicated
 * here.
 */
interface ProviderDef {
  id: string;
  label: string;
  desc: string;
  dep?: string;
  get: (c: Config) => boolean;
  set: (c: Config, v: boolean) => Config;
}

const PROVIDERS: ProviderDef[] = [
  { id: "apps",  label: "Applications", desc: "Find installed applications.",
    get: c => c.providers.apps,  set: (c, v) => ({ ...c, providers: { ...c.providers, apps: v } }) },
  { id: "files", label: "Files", desc: "Search file and folder names in your chosen directories.",
    get: c => c.providers.files, set: (c, v) => ({ ...c, providers: { ...c.providers, files: v } }) },
  { id: "calc",  label: "Calculator", desc: "Type a math expression to get the answer inline.",
    get: c => c.providers.calc,  set: (c, v) => ({ ...c, providers: { ...c.providers, calc: v } }) },
  { id: "dict",  label: "Dictionary", desc: "Word definitions via dict.", dep: "dict",
    get: c => c.dict.enabled,     set: (c, v) => ({ ...c, dict: { ...c.dict, enabled: v } }) },
];

function CopyHint({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      className="settings-copy-hint"
      title="Copy install command"
      onClick={() => {
        navigator.clipboard?.writeText(text).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
        }).catch(() => {});
      }}
    >
      <code>{text}</code>
      <span className="settings-copy-hint-icon">{copied ? "✓" : "⧉"}</span>
    </button>
  );
}

export default function ProvidersSection({ config, onChange }: Props) {
  const deps = useDeps();

  return (
    <div className="settings-section">
      <SectionHeader title="Providers" desc="Enable or disable individual search sources." />

      <SettingsGroup>
        {PROVIDERS.map(p => {
          const enabled = p.get(config);
          const dep = p.dep ? deps?.find(d => d.id === p.dep) : undefined;
          const missing = enabled && dep && !dep.available;
          return (
            <SettingsField
              key={p.id}
              name={p.label}
              desc={p.desc}
              warn={missing && (
                <div className="settings-dep-inline-warn">
                  ⚠ Enabled but <code>{dep!.label}</code> is missing. Install <code>{dep!.install_hint}</code>
                </div>
              )}
            >
              <Toggle label={p.label} checked={enabled} onChange={v => onChange(p.set(config, v))} />
            </SettingsField>
          );
        })}
      </SettingsGroup>

      <SettingsGroup
        title="System dependencies"
        desc="Optional tools that power individual features. A missing tool disables only its feature."
      >
        {deps === null ? (
          <div className="settings-dep-empty">Checking…</div>
        ) : (
          deps.map(d => (
            <div className="settings-dep-row" key={d.id}>
              <span className={`settings-dep-dot${d.available ? " ok" : " missing"}`} />
              <span className="settings-dep-feature">{d.feature}</span>
              {d.available ? (
                <span className="settings-dep-tool">{d.label} ✓</span>
              ) : (
                <span className="settings-dep-tool settings-dep-tool--missing">
                  {d.label} missing — <CopyHint text={d.install_hint} />
                </span>
              )}
            </div>
          ))
        )}
      </SettingsGroup>
    </div>
  );
}
