import { Config } from "../../types";
import Toggle from "./Toggle";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

export default function DebugSection({ config, onChange }: Props) {
  const set = (patch: Partial<Config["debug"]>) =>
    onChange({ ...config, debug: { ...config.debug, ...patch } });

  return (
    <div>
      <div className="settings-section-header">
        <div className="settings-section-name">Debug</div>
        <div className="settings-section-desc">Diagnostic output written to stderr. Useful when troubleshooting search quality or watcher issues.</div>
      </div>

      <div className="settings-field">
        <div className="settings-field-label">
          <div className="settings-field-name">Log match scores</div>
          <div className="settings-field-desc">Print fuzzy match scores and thresholds for every candidate to stderr</div>
        </div>
        <div className="settings-field-control">
          <Toggle label="Log match scores" checked={config.debug.log_scores} onChange={v => set({ log_scores: v })} />
        </div>
      </div>

      <div className="settings-field">
        <div className="settings-field-label">
          <div className="settings-field-name">Log watcher events</div>
          <div className="settings-field-desc">Print filesystem watcher events and index update decisions to stderr</div>
        </div>
        <div className="settings-field-control">
          <Toggle label="Log watcher events" checked={config.debug.log_watcher} onChange={v => set({ log_watcher: v })} />
        </div>
      </div>
    </div>
  );
}
