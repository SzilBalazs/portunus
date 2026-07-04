import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ExtensionSettingSpec } from "../../../types";
import SettingsField from "../SettingsField";
import Toggle from "../Toggle";
import Select from "../Select";
import NumberStepper from "../NumberStepper";
import TextInput from "../TextInput";

interface Props {
  extension: string;
  schema: ExtensionSettingSpec[];
  values: Record<string, unknown>;
}

const SAVE_DELAY_MS = 800;

/**
 * Renders an extension's declared settings schema with the shared control
 * primitives. Values live in local state (seeded from the backend snapshot)
 * and save debounced via `set_extension_settings`, which hot-reloads just
 * that extension. Deliberately NOT routed through the Config autosave - these
 * are extension-owned values with their own validation.
 *
 * A dirty flag guards against the post-reload `list_extensions` refresh
 * clobbering edits the debounce hasn't flushed yet.
 */
export default function ExtensionSettingsForm({ extension, schema, values }: Props) {
  const [local, setLocal] = useState<Record<string, unknown>>(values);
  const dirtyRef = useRef(false);
  const timerRef = useRef<number | undefined>(undefined);

  useEffect(() => {
    if (!dirtyRef.current) setLocal(values);
  }, [values]);

  const set = (key: string, value: unknown) => {
    dirtyRef.current = true;
    const next = { ...local, [key]: value };
    setLocal(next);
    window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(() => {
      dirtyRef.current = false;
      invoke("set_extension_settings", { name: extension, values: next })
        .catch(e => console.error(`[extensions] settings save failed: ${e}`));
    }, SAVE_DELAY_MS);
  };
  useEffect(() => () => window.clearTimeout(timerRef.current), []);

  if (schema.length === 0) return null;
  return (
    <div className="settings-ext-form">
      {schema.map(spec => (
        <SettingsField key={spec.key} name={spec.label} desc={spec.description || undefined}>
          {spec.type === "bool" && (
            <Toggle label={spec.label} checked={Boolean(local[spec.key])} onChange={v => set(spec.key, v)} />
          )}
          {spec.type === "string" && (
            <TextInput
              label={spec.label}
              value={String(local[spec.key] ?? "")}
              placeholder={spec.placeholder || undefined}
              width={180}
              onChange={v => set(spec.key, v)}
            />
          )}
          {spec.type === "number" && (
            <NumberStepper
              label={spec.label}
              value={Number(local[spec.key] ?? 0)}
              min={spec.min ?? undefined}
              max={spec.max ?? undefined}
              step={spec.step ?? undefined}
              onChange={v => set(spec.key, v)}
            />
          )}
          {spec.type === "select" && (
            <Select
              options={spec.options.map(label => ({ label }))}
              value={String(local[spec.key] ?? spec.options[0] ?? "")}
              onChange={v => set(spec.key, v)}
            />
          )}
        </SettingsField>
      ))}
    </div>
  );
}
