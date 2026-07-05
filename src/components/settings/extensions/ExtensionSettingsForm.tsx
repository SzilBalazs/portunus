import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ExtensionSettingSpec } from "../../../types";
import SettingsField from "../SettingsField";
import Toggle from "../Toggle";
import Select from "../Select";
import NumberStepper from "../NumberStepper";
import TextInput from "../TextInput";
import SecretSettingInput from "./SecretSettingInput";

interface Props {
  extension: string;
  schema: ExtensionSettingSpec[];
  values: Record<string, unknown>;
  /** Secret keys with a stored keyring value. */
  secretsSet: string[];
  /** Whether a Secret Service daemon is reachable. */
  secretsAvailable: boolean;
  /** Refresh callback after a secret is set/cleared. */
  onChanged: () => void;
}

const SAVE_DELAY_MS = 800;

/** Backend rejections come back as `setting "key": message`. */
const FIELD_ERROR_RE = /^setting "([a-z0-9_]+)": (.*)$/s;

/** Client-side pre-check mirroring the backend's number bounds - typed values
 * can escape NumberStepper clamping. Returns per-key errors, empty when clean. */
function precheck(schema: ExtensionSettingSpec[], values: Record<string, unknown>): Record<string, string> {
  const errors: Record<string, string> = {};
  for (const spec of schema) {
    if (spec.type !== "number") continue;
    const v = values[spec.key];
    if (v === undefined || v === null || v === "") continue;
    const n = Number(v);
    if (!Number.isFinite(n)) errors[spec.key] = "must be a number";
    else if (spec.min != null && n < spec.min) errors[spec.key] = `must be at least ${spec.min}`;
    else if (spec.max != null && n > spec.max) errors[spec.key] = `must be at most ${spec.max}`;
  }
  return errors;
}

/**
 * Renders an extension's declared settings schema with the shared control
 * primitives. Values live in local state (seeded from the backend snapshot)
 * and save debounced via `set_extension_settings`, which hot-reloads just
 * that extension. Deliberately NOT routed through the Config autosave - these
 * are extension-owned values with their own validation.
 *
 * A dirty flag guards against the post-reload `list_extensions` refresh
 * clobbering edits the debounce hasn't flushed yet; it also stays set when a
 * save is rejected, so the invalid draft remains visible next to its error.
 */
export default function ExtensionSettingsForm({ extension, schema, values, secretsSet, secretsAvailable, onChanged }: Props) {
  const [local, setLocal] = useState<Record<string, unknown>>(values);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [formError, setFormError] = useState<string | null>(null);
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
      const pre = precheck(schema, next);
      setErrors(pre);
      setFormError(null);
      if (Object.keys(pre).length > 0) return; // stay dirty, keep the draft
      dirtyRef.current = false;
      // Secrets go through the dedicated commands, never this config path -
      // strip them defensively (the backend rejects them anyway).
      const secretKeys = new Set(schema.filter(s => s.type === "secret").map(s => s.key));
      const payload = Object.fromEntries(Object.entries(next).filter(([k]) => !secretKeys.has(k)));
      invoke("set_extension_settings", { name: extension, values: payload })
        .catch(e => {
          dirtyRef.current = true;
          const m = FIELD_ERROR_RE.exec(String(e));
          if (m) setErrors({ [m[1]]: m[2] });
          else setFormError(String(e));
        });
    }, SAVE_DELAY_MS);
  };
  useEffect(() => () => window.clearTimeout(timerRef.current), []);

  if (schema.length === 0) return null;
  return (
    <div className="settings-ext-form">
      {formError && (
        <div className="settings-dep-inline-warn">{formError}</div>
      )}
      {schema.map(spec => (
        <SettingsField key={spec.key} name={spec.label} desc={spec.description || undefined} stacked={spec.type === "secret" || spec.type === "string"}>
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
          {spec.type === "secret" && (
            <SecretSettingInput
              extension={extension}
              settingKey={spec.key}
              placeholder={spec.placeholder || undefined}
              isSet={secretsSet.includes(spec.key)}
              available={secretsAvailable}
              onChanged={onChanged}
            />
          )}
          {errors[spec.key] && (
            <div className="settings-ext-field-error">{errors[spec.key]}</div>
          )}
        </SettingsField>
      ))}
    </div>
  );
}
