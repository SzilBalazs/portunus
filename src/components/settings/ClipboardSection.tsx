import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Config, DepStatus } from "../../types";
import Toggle from "./Toggle";
import Select from "./Select";
import NumberField from "./NumberField";

const PASTE_MODES: { label: string; value: "auto" | "copy" }[] = [
  { label: "Paste into focused app (auto)", value: "auto" },
  { label: "Copy to clipboard only", value: "copy" },
];

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

export default function ClipboardSection({ config, onChange }: Props) {
  const [deps, setDeps] = useState<DepStatus[] | null>(null);
  useEffect(() => {
    invoke<DepStatus[]>("check_dependencies").then(setDeps).catch(() => setDeps([]));
  }, []);

  const depById = (id: string) => deps?.find(d => d.id === id);

  return (
    <div>
      <div className="settings-section-header">
        <div className="settings-section-name">Clipboard history</div>
        <div className="settings-section-desc">
          The clipboard browser (<code>clip</code> or <code>portunus --clipboard</code>).
        </div>
      </div>

      <div className="settings-field">
        <div className="settings-field-label">
          <div className="settings-field-name">On Enter</div>
          <div className="settings-field-desc">
            Auto paste types Ctrl+V into the previously focused window.
          </div>
          {config.clipboard.paste_mode === "auto" && depById("wtype") && !depById("wtype")!.available && (
            <div className="settings-dep-inline-warn">
              ⚠ Auto paste needs <code>wtype</code>; without it Enter falls back to copy-only. Install <code>wtype</code>
            </div>
          )}
        </div>
        <div className="settings-field-control">
          <Select
            options={PASTE_MODES.map(m => ({ label: m.label }))}
            value={PASTE_MODES.find(m => m.value === config.clipboard.paste_mode)?.label ?? PASTE_MODES[0].label}
            onChange={label => {
              const mode = PASTE_MODES.find(m => m.label === label)?.value ?? "auto";
              onChange({ ...config, clipboard: { ...config.clipboard, paste_mode: mode } });
            }}
          />
        </div>
      </div>

      <NumberField
        label="Max entries"
        desc="How many history entries the browser loads."
        value={config.clipboard.max_entries}
        min={10}
        max={750}
        step={10}
        width={70}
        onChange={v => onChange({ ...config, clipboard: { ...config.clipboard, max_entries: v } })}
      />

      <div className="settings-field">
        <div className="settings-field-label">
          <div className="settings-field-name">OCR copied images</div>
          <div className="settings-field-desc">
            Run OCR on copied images so their visible text is searchable in the browser. Each image is OCR'd once and cached.
          </div>
        </div>
        <div className="settings-field-control">
          <Toggle
            label="OCR copied images"
            checked={config.clipboard.ocr_images}
            onChange={v => onChange({ ...config, clipboard: { ...config.clipboard, ocr_images: v } })}
          />
        </div>
      </div>

      {config.clipboard.ocr_images && (
        <div className="settings-field">
          <div className="settings-field-label">
            <div className="settings-field-name">OCR language</div>
            <div className="settings-field-desc">
              Tesseract language code for clipboard images (independent of the Content tab). English (eng) is bundled; other languages need the matching tesseract-data-&lt;lang&gt; data. Combine with <code style={{ fontFamily: "monospace" }}>+</code> (e.g. <code style={{ fontFamily: "monospace" }}>eng+hun</code>).
            </div>
          </div>
          <div className="settings-field-control">
            <input
              className="settings-text-input"
              value={config.clipboard.ocr_language}
              onChange={e => onChange({ ...config, clipboard: { ...config.clipboard, ocr_language: e.target.value } })}
              placeholder="eng"
              style={{ width: 80 }}
            />
          </div>
        </div>
      )}
    </div>
  );
}
