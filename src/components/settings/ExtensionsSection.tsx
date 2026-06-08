import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTauriListener } from "../../hooks/useTauriListener";
import { Config, ExtensionInfo } from "../../types";
import { WarnIcon } from "../../icons";
import Toggle from "./Toggle";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

function fmtInterval(secs: number): string {
  if (secs % 3600 === 0) return `${secs / 3600}h`;
  if (secs % 60 === 0) return `${secs / 60}m`;
  return `${secs}s`;
}

/** Human summary of what an extension may touch - shown BEFORE first enable. */
function PermissionChips({ info }: { info: ExtensionInfo }) {
  if (!info.permissions) return null;
  const chips: string[] = [];
  if (info.permissions.network.length > 0)
    chips.push(`network: ${info.permissions.network.join(", ")}`);
  if (info.permissions.kv) chips.push("storage");
  if (info.permissions.clipboard) chips.push("clipboard");
  if (info.permissions.open_url) chips.push("open urls");
  if (chips.length === 0) chips.push("no permissions");
  if (info.background_interval_secs != null)
    chips.push(`background: every ${fmtInterval(info.background_interval_secs)}`);
  return (
    <div className="settings-field-desc">
      {chips.map(c => (
        <code key={c} style={{ marginRight: 6 }}>{c}</code>
      ))}
    </div>
  );
}

export default function ExtensionsSection({ config, onChange }: Props) {
  const [exts, setExts] = useState<ExtensionInfo[] | null>(null);

  const refresh = useCallback(() => {
    invoke<ExtensionInfo[]>("list_extensions")
      .then(next =>
        // Skip the state update when nothing changed - a no-op refresh after
        // Rescan would otherwise re-render every row (visible as a flash).
        setExts(prev =>
          prev && JSON.stringify(prev) === JSON.stringify(next) ? prev : next,
        ),
      )
      .catch(() => setExts([]));
  }, []);
  useEffect(refresh, [refresh]);
  // Fired by the backend when an extension rebuild completes (Rescan button,
  // --reload-extensions, config toggle) - refresh loaded/error states then,
  // instead of guessing with a timer.
  useTauriListener("search-invalidated", refresh, [refresh]);
  // Runtime errors happen while the user is in the LAUNCHER (a search call
  // traps); no event reaches this window, so refresh whenever it regains
  // focus - same pattern Settings.tsx uses for config reloads.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (focused) refresh();
      })
      .then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, [refresh]);

  const setEnabled = (name: string, value: boolean) =>
    onChange({
      ...config,
      extensions: {
        ...config.extensions,
        enabled: { ...config.extensions.enabled, [name]: value },
      },
    });

  const rescan = () => {
    // Rebuilds in a background thread; the search-invalidated listener above
    // refreshes the list when it completes.
    invoke("rescan_extensions").catch(() => {});
  };

  return (
    <div>
      <div className="settings-section-header">
        <div className="settings-section-name">Extensions</div>
        <div className="settings-section-desc">
          WASM extensions from <code>~/.local/share/portunus/extensions/</code>. New
          extensions stay disabled until you review their permissions and enable them.
        </div>
      </div>

      {exts === null && <div className="settings-field-desc">Scanning…</div>}
      {exts?.length === 0 && (
        <div className="settings-field-desc">
          No extensions installed. Drop a folder with <code>manifest.toml</code> +{" "}
          <code>extension.wasm</code> into the extensions directory, then Rescan.
        </div>
      )}

      {exts?.map(info => {
        // Live toggle state comes from config (updates instantly on click);
        // `info` is a backend snapshot and only refreshes on rescan.
        const enabled = config.extensions.enabled[info.name] ?? false;
        const isNew = !(info.name in config.extensions.enabled);
        return (
          <div className="settings-field" key={info.name}>
            <div className="settings-field-label">
              <div className="settings-field-name">
                {info.name}
                {info.version && <span className="settings-field-desc"> v{info.version}</span>}
                {isNew && <span className="settings-field-desc"> (new, review &amp; enable)</span>}
              </div>
              {info.description && <div className="settings-field-desc">{info.description}</div>}
              <PermissionChips info={info} />
              {info.error && (
                <div className="settings-dep-inline-warn"><WarnIcon />{info.error}</div>
              )}
              {info.benched && (
                <div className="settings-dep-inline-warn">
                  <WarnIcon />Disabled for this session after repeated failures. Fix and Rescan.
                </div>
              )}
            </div>
            <div className="settings-field-control">
              <Toggle
                label={info.name}
                checked={enabled}
                onChange={v => setEnabled(info.name, v)}
              />
            </div>
          </div>
        );
      })}

      <div className="settings-field">
        <div className="settings-field-label">
          <div className="settings-field-name">Rescan</div>
          <div className="settings-field-desc">
            Re-discover the extensions directory and reload wasm files (also:{" "}
            <code>portunus --reload-extensions</code>).
          </div>
        </div>
        <div className="settings-field-control">
          <button className="settings-reindex-apply" onClick={rescan}>Rescan</button>
        </div>
      </div>
    </div>
  );
}
