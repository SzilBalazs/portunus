import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Config, DepStatus } from "../../types";
import Toggle from "./Toggle";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

interface ProviderDef {
  key: keyof Config["providers"];
  label: string;
  desc: string;
  // Dependency id (from check_dependencies) this provider needs to function.
  dep?: string;
}

const PROVIDERS: ProviderDef[] = [
  { key: "apps",  label: "Applications", desc: "Search .desktop application entries" },
  { key: "files", label: "Files",        desc: "Indexed file search" },
  { key: "calc",  label: "Calculator",   desc: "Inline math expression evaluator" },
];

export default function ProvidersSection({ config, onChange }: Props) {
  const set = (key: keyof Config["providers"], value: boolean) =>
    onChange({ ...config, providers: { ...config.providers, [key]: value } });

  const [deps, setDeps] = useState<DepStatus[] | null>(null);
  useEffect(() => {
    invoke<DepStatus[]>("check_dependencies").then(setDeps).catch(() => setDeps([]));
  }, []);

  const depById = (id: string) => deps?.find(d => d.id === id);

  return (
    <div>
      <div className="settings-section-header">
        <div className="settings-section-name">Providers</div>
        <div className="settings-section-desc">Enable or disable individual search providers.</div>
      </div>

      {PROVIDERS.map(({ key, label, desc, dep }) => {
        const enabled = config.providers[key];
        const status = dep ? depById(dep) : undefined;
        const missing = enabled && status && !status.available;
        return (
          <div className="settings-field" key={key}>
            <div className="settings-field-label">
              <div className="settings-field-name">{label}</div>
              <div className="settings-field-desc">{desc}</div>
              {missing && (
                <div className="settings-dep-inline-warn">
                  ⚠ Enabled but <code>{status!.label}</code> is missing. Install <code>{status!.install_hint}</code>
                </div>
              )}
            </div>
            <div className="settings-field-control">
              <Toggle label={label} checked={enabled} onChange={v => set(key, v)} />
            </div>
          </div>
        );
      })}

      <div className="settings-deps">
        <div className="settings-deps-title">System dependencies</div>
        <div className="settings-field-desc" style={{ marginBottom: 10 }}>
          Optional tools that power individual features. Missing tools disable only their feature.
        </div>
        {deps === null ? (
          <div className="settings-field-desc">Checking…</div>
        ) : (
          deps.map(d => (
            <div className="settings-dep-row" key={d.id}>
              <span className={`settings-dep-dot${d.available ? " ok" : " missing"}`} />
              <span className="settings-dep-feature">{d.feature}</span>
              <span className="settings-dep-tool">
                {d.available
                  ? <>{d.label} ✓</>
                  : <>{d.label} missing. Install <code>{d.install_hint}</code></>}
              </span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
