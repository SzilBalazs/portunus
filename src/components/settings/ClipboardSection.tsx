import { Config } from "../../types";
import Toggle from "./Toggle";
import Select from "./Select";
import SectionHeader from "./SectionHeader";
import SettingsGroup from "./SettingsGroup";
import SettingsField from "./SettingsField";
import NumberStepper from "./NumberStepper";
import { useDep } from "./DepsContext";

const PASTE_MODES: { label: string; value: "auto" | "copy" }[] = [
  { label: "Paste into focused app (auto)", value: "auto" },
  { label: "Copy to clipboard only", value: "copy" },
];

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

export default function ClipboardSection({ config, onChange }: Props) {
  const set = (patch: Partial<Config["clipboard"]>) =>
    onChange({ ...config, clipboard: { ...config.clipboard, ...patch } });

  const wtype = useDep("wtype");
  const pasteNeedsWtype = config.clipboard.paste_mode === "auto" && wtype && !wtype.available;

  return (
    <div className="settings-section">
      <SectionHeader
        title="Clipboard history"
        desc="Browse and reuse your clipboard history."
      />

      <SettingsGroup>
        <SettingsField
          name="On Enter"
          desc="Auto paste types Ctrl+V into the previously focused window."
          warn={pasteNeedsWtype && (
            <div className="settings-dep-inline-warn">
              ⚠ Auto paste needs <code>wtype</code>; without it Enter falls back to copy-only. Install <code>wtype</code>
            </div>
          )}
        >
          <Select
            options={PASTE_MODES.map(m => ({ label: m.label }))}
            value={PASTE_MODES.find(m => m.value === config.clipboard.paste_mode)?.label ?? PASTE_MODES[0].label}
            onChange={label => set({ paste_mode: PASTE_MODES.find(m => m.label === label)?.value ?? "auto" })}
          />
        </SettingsField>

        <SettingsField name="Max entries" desc="How many history entries the browser loads.">
          <NumberStepper label="Max entries" value={config.clipboard.max_entries} min={10} max={750} step={10} width={70} onChange={max_entries => set({ max_entries })} />
        </SettingsField>

        <SettingsField
          name="OCR copied images"
          desc={<>Run OCR on copied images so their visible text is searchable. Each image is OCR'd once and cached. Uses the OCR language set under <strong>Content</strong>.</>}
        >
          <Toggle label="OCR copied images" checked={config.clipboard.ocr_images} onChange={ocr_images => set({ ocr_images })} />
        </SettingsField>
      </SettingsGroup>
    </div>
  );
}
