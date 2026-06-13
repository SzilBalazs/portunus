import { Config } from "../../types";
import ThemeGrid from "./ThemeGrid";
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

// Labels are user-facing; values are the config enum. "Smooth" replaces the raw
// "FLIP" jargon, but the stored value stays "flip" for backward compatibility.
const ANIM_OPTIONS = [
  { label: "Off",    value: "off"   },
  { label: "Slide",  value: "slide" },
  { label: "Smooth", value: "flip"  },
] as const;

function animLabel(v: Config["appearance"]["animate_results"]): string {
  return ANIM_OPTIONS.find(o => o.value === v)?.label ?? "Slide";
}

export default function AppearanceSection({ config, onChange }: Props) {
  const set = (patch: Partial<Config["appearance"]>) =>
    onChange({ ...config, appearance: { ...config.appearance, ...patch } });

  const { theme, font_size, animate_results, show_metadata, accent_bleed, slide_selection } = config.appearance;

  return (
    <div className="settings-section">
      <SectionHeader title="Appearance" desc="Theme, scale, and launcher visuals." />

      <div className="settings-group-block">
        <div className="settings-group-title">Theme</div>
        <ThemeGrid value={theme} onSelect={id => set({ theme: id })} />
      </div>

      <SettingsGroup title="Display">
        <SettingsField name="Interface scale" desc="Scale the entire launcher UI proportionally.">
          <Slider
            label="Interface scale"
            value={font_size}
            min={11} max={18} step={1}
            format={v => `${v}px`}
            commitOnRelease
            onChange={v => set({ font_size: v })}
          />
        </SettingsField>

        <SettingsField
          name="Result animations"
          desc="How result rows animate: none, a slide-in entrance, or smoothly repositioning retained rows."
        >
          <Select
            options={ANIM_OPTIONS.map(o => ({ label: o.label }))}
            value={animLabel(animate_results)}
            onChange={label => {
              const opt = ANIM_OPTIONS.find(o => o.label === label);
              if (opt) set({ animate_results: opt.value });
            }}
          />
        </SettingsField>

        <SettingsField name="File metadata" desc="Show the modified/created row in file previews.">
          <Toggle label="File metadata" checked={show_metadata ?? true} onChange={v => set({ show_metadata: v })} />
        </SettingsField>

        <SettingsField name="Accent bleed" desc="Tint the selection and preview with the app's icon color.">
          <Toggle label="Accent bleed" checked={accent_bleed ?? true} onChange={v => set({ accent_bleed: v })} />
        </SettingsField>

        <SettingsField name="Sliding selection" desc="Glide the highlight between rows as you navigate.">
          <Toggle label="Sliding selection" checked={slide_selection ?? true} onChange={v => set({ slide_selection: v })} />
        </SettingsField>
      </SettingsGroup>
    </div>
  );
}
