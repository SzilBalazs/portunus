import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTauriListener } from "../../hooks/useTauriListener";
import { AppUpdateStatus, Config } from "../../types";
import { RefreshIcon } from "../../icons";
import Badge from "./Badge";
import NumberStepper from "./NumberStepper";
import SectionHeader from "./SectionHeader";
import SettingsField from "./SettingsField";
import SettingsGroup from "./SettingsGroup";
import Toggle from "./Toggle";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

/** Renders the persisted check timestamp as a short relative age. */
function agoLabel(checkedAt: number): string {
  const secs = Math.max(0, Math.floor(Date.now() / 1000) - checkedAt);
  if (secs < 90) return "just now";
  const mins = Math.round(secs / 60);
  if (mins < 90) return `${mins} min ago`;
  const hours = Math.round(mins / 60);
  if (hours < 36) return `${hours} h ago`;
  return `${Math.round(hours / 24)} d ago`;
}

export default function AboutSection({ config, onChange }: Props) {
  const setGeneral = (patch: Partial<Config["general"]>) =>
    onChange({ ...config, general: { ...config.general, ...patch } });

  const [status, setStatus] = useState<AppUpdateStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [checkErr, setCheckErr] = useState<string | null>(null);

  // Cache-only read: no network, so it is safe on mount and on every event.
  const fetchStatus = useCallback(() => {
    invoke<AppUpdateStatus>("app_update_status").then(setStatus).catch(() => {});
  }, []);
  useEffect(fetchStatus, [fetchStatus]);
  useTauriListener("app-update-available", fetchStatus, [fetchStatus]);

  // Forces a fetch regardless of the interval or the toggle - the click is the
  // consent. The outcome shows up in the badge + "last checked" line, so only a
  // failure needs its own text; it goes in the field's warn slot (wraps freely)
  // rather than the button label, which must keep a fixed width. It stays put
  // until the next click clears it - no timer to cancel on unmount.
  const checkNow = async () => {
    setChecking(true);
    setCheckErr(null);
    try {
      setStatus(await invoke<AppUpdateStatus>("app_update_check"));
    } catch (e) {
      setCheckErr(String(e));
    }
    setChecking(false);
  };

  const latest = status?.latest ?? null;
  const checked = (status?.checked_at ?? 0) > 0;

  return (
    <div className="settings-section">
      <SectionHeader title="About" desc="Version and release updates." />

      <SettingsGroup>
        <SettingsField
          name={
            <>
              Portunus v{status?.current_version ?? "…"}
              {status?.update_available && latest && <> <Badge tone="update">v{latest.version} available</Badge></>}
              {checked && !status?.update_available && <> <Badge tone="success">Up to date</Badge></>}
            </>
          }
          desc={
            checked
              ? `Last checked ${agoLabel(status!.checked_at)}.`
              : "No release check has run yet."
          }
          warn={checkErr && <div className="settings-dep-inline-warn">⚠ Update check failed: {checkErr}</div>}
        >
          <div className="settings-btn-row">
            {status?.update_available && latest && (
              <button
                className="settings-btn-primary"
                onClick={() => openUrl(latest.url)}
                title={latest.url}
              >
                View release
              </button>
            )}
            <button
              className="settings-btn-secondary"
              onClick={checkNow}
              disabled={checking}
              data-busy={checking || undefined}
            >
              <RefreshIcon />
              Check now
            </button>
          </div>
        </SettingsField>

        <SettingsField
          name="Check for updates"
          desc="Ask GitHub for the latest release on an interval. Notify only — Portunus never downloads or installs anything. Turning this off stops all network contact."
        >
          <Toggle
            label="Check for updates"
            checked={config.general.check_for_updates}
            onChange={check_for_updates => setGeneral({ check_for_updates })}
          />
        </SettingsField>

        <SettingsField
          name="Check interval"
          desc="Hours between automatic checks. The last check time is remembered across restarts."
        >
          <NumberStepper
            label="Check interval"
            value={config.general.update_check_interval_hours}
            min={1}
            max={168}
            suffix="hours"
            onChange={update_check_interval_hours => setGeneral({ update_check_interval_hours })}
          />
        </SettingsField>
      </SettingsGroup>
    </div>
  );
}
