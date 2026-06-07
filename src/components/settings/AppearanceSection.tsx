import { Config } from "../../types";
import ThemeGrid from "./ThemeGrid";
import Toggle from "./Toggle";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

export default function AppearanceSection({ config, onChange }: Props) {
  const set = (patch: Partial<Config["appearance"]>) =>
    onChange({ ...config, appearance: { ...config.appearance, ...patch } });

  const { theme, font_size, animate_results } = config.appearance;

  return (
    <div>
      <div className="settings-section-header">
        <div className="settings-section-name">Appearance</div>
        <div className="settings-section-desc">Theme and display settings.</div>
      </div>

      <div style={{ marginBottom: 24 }}>
        <div style={{ fontSize: 12, fontWeight: 600, color: "var(--fg-mute)", letterSpacing: "0.08em", textTransform: "uppercase", marginBottom: 2 }}>
          Theme
        </div>
        <ThemeGrid value={theme} onSelect={(id) => set({ theme: id })} />
      </div>

      <div className="settings-field">
        <div className="settings-field-label">
          <div className="settings-field-name">Zoom</div>
          <div className="settings-field-desc">Scale the entire UI proportionally</div>
        </div>
        <div className="settings-field-control">
          <div className="settings-number-wrap">
            <button
              className="settings-number-btn"
              onClick={() => set({ font_size: Math.max(11, font_size - 1) })}
            >−</button>
            <input
              type="number"
              className="settings-number-input"
              value={font_size}
              min={11} max={18} step={1}
              onChange={e => {
                const v = parseInt(e.target.value);
                if (!isNaN(v) && v >= 11 && v <= 18) set({ font_size: v });
              }}
            />
            <button
              className="settings-number-btn"
              onClick={() => set({ font_size: Math.min(18, font_size + 1) })}
            >+</button>
          </div>
        </div>
      </div>

      <div className="settings-field">
        <div className="settings-field-label">
          <div className="settings-field-name">Result animations</div>
          <div className="settings-field-desc">Slide-in animation when results appear</div>
        </div>
        <div className="settings-field-control">
          <Toggle
            label="Result animations"
            checked={animate_results ?? true}
            onChange={v => set({ animate_results: v })}
          />
        </div>
      </div>
    </div>
  );
}
