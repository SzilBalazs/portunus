import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ExtensionInfo, InstallPreview, UpdateCheck } from "../../../types";
import { WarnIcon } from "../../../icons";
import Toggle from "../Toggle";
import Badge from "../Badge";
import Modal from "../Modal";
import PermissionChips from "./PermissionChips";
import ExtensionSettingsForm from "./ExtensionSettingsForm";
import ExtensionLogs from "./ExtensionLogs";
import InstallExtensionDialog from "./InstallExtensionDialog";

interface Props {
  info: ExtensionInfo;
  enabled: boolean;
  /** Not yet present in config - never enabled, still needs review. */
  isNew: boolean;
  onSetEnabled: (v: boolean) => void;
  onChanged: () => void;
}

/**
 * One installed extension: a collapsed header row (name, version, badges,
 * enable toggle) that expands into the full detail card - permissions, origin
 * and hash, the schema-driven settings form, a log viewer, and the
 * update/uninstall actions.
 */
export default function ExtensionCard({ info, enabled, isNew, onSetEnabled, onChanged }: Props) {
  const [expanded, setExpanded] = useState(false);
  const [updateState, setUpdateState] = useState<"idle" | "checking" | "current">("idle");
  const [updatePreview, setUpdatePreview] = useState<InstallPreview | null>(null);
  const [confirmUninstall, setConfirmUninstall] = useState(false);

  const checkUpdate = () => {
    setUpdateState("checking");
    invoke<UpdateCheck>("check_extension_update", { name: info.name })
      .then(res => {
        if (res.preview) {
          setUpdatePreview(res.preview);
          setUpdateState("idle");
        } else {
          setUpdateState("current");
          window.setTimeout(() => setUpdateState("idle"), 2500);
        }
      })
      .catch(e => {
        console.error(`[extensions] update check failed: ${e}`);
        setUpdateState("idle");
      });
  };

  const uninstall = () => {
    invoke("uninstall_extension", { name: info.name })
      .then(onChanged)
      .catch(e => console.error(`[extensions] uninstall failed: ${e}`));
    setConfirmUninstall(false);
  };

  const reconsent = () => {
    invoke("consent_extension_permissions", { name: info.name })
      .then(() => { invoke("rescan_extensions").catch(() => {}); onChanged(); })
      .catch(e => console.error(`[extensions] consent failed: ${e}`));
  };

  // No parsed manifest = the extension cannot load; the toggle would be a lie.
  const broken = !info.permissions;
  // Collapsed cards still need to say WHY they're red - surface the first
  // error line without requiring an expand.
  const subline = info.error ?? (info.description || null);

  return (
    <div className={`settings-ext-card${expanded ? " expanded" : ""}`}>
      <div className="settings-ext-card-header" onClick={() => setExpanded(e => !e)}>
        <svg className="settings-ext-chevron" width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
          <path d="M3.5 2l3 3-3 3" />
        </svg>
        <div className="settings-ext-card-title">
          <div className="settings-ext-card-titlerow">
            <span className="settings-ext-card-name">{info.name}</span>
            {info.version && <span className="settings-ext-version">v{info.version}</span>}
            {info.dev && <Badge tone="dev">dev</Badge>}
            {isNew && !broken && <Badge tone="new">new — review &amp; enable</Badge>}
            {info.needs_reconsent && <Badge tone="update">permissions changed</Badge>}
            {(info.error || info.benched) && !info.needs_reconsent && <Badge tone="error">error</Badge>}
          </div>
          {subline && (
            <div className={`settings-ext-card-sub${info.error ? " is-error" : ""}`}>{subline}</div>
          )}
        </div>
        <span className="settings-ext-card-spacer" />
        <span onClick={e => e.stopPropagation()} title={broken ? "Fix the error below to enable" : undefined}>
          <Toggle label={info.name} checked={enabled && !broken} disabled={broken} onChange={onSetEnabled} />
        </span>
      </div>

      {expanded && (
        <div className="settings-ext-card-body">
          {info.description && <div className="settings-field-desc">{info.description}</div>}
          {info.triggers.length > 0 && (
            <div className="settings-field-desc">
              Trigger: {info.triggers.map(t => <code key={t}>{t}&nbsp;</code>)}
              — type it in the launcher to search this extension.
            </div>
          )}
          <PermissionChips permissions={info.permissions} backgroundIntervalSecs={info.background_interval_secs} />

          {info.error && (
            <div className="settings-dep-inline-warn"><WarnIcon />{info.error}</div>
          )}
          {info.benched && (
            <div className="settings-dep-inline-warn"><WarnIcon />Disabled for this session after repeated failures. Fix and Rescan.</div>
          )}
          {info.needs_reconsent && (
            <div className="settings-ext-reconsent">
              <button className="settings-btn-primary" onClick={reconsent}>Review &amp; allow new permissions</button>
            </div>
          )}

          <div className="settings-ext-origin">
            {info.dev
              ? <>Linked working directory (<code>portunus ext dev</code>)</>
              : info.origin === "url"
                ? <>Installed from <code className="settings-ext-origin-url">{info.origin_url}</code></>
                : <>Installed locally</>}
            {info.homepage && <> · <a href={info.homepage} target="_blank" rel="noreferrer">{info.homepage}</a></>}
          </div>

          <ExtensionSettingsForm extension={info.name} schema={info.settings_schema} values={info.settings_values} />

          <ExtensionLogs extension={info.name} />

          <div className="settings-ext-card-actions">
            {info.origin === "url" && !info.dev && (
              <button className="settings-btn-secondary" onClick={checkUpdate} disabled={updateState === "checking"}>
                {updateState === "checking" ? "Checking…" : updateState === "current" ? "Up to date" : "Check for update"}
              </button>
            )}
            <span className="settings-ext-card-spacer" />
            <button className="settings-btn-danger" onClick={() => setConfirmUninstall(true)}>
              {info.dev ? "Unlink" : "Uninstall"}
            </button>
          </div>
        </div>
      )}

      {updatePreview && (
        <InstallExtensionDialog
          initialPreview={updatePreview}
          onClose={() => setUpdatePreview(null)}
          onInstalled={onChanged}
        />
      )}

      {confirmUninstall && (
        <Modal
          title={info.dev ? `Unlink ${info.name}?` : `Uninstall ${info.name}?`}
          onClose={() => setConfirmUninstall(false)}
          footer={
            <>
              <button className="settings-btn-secondary" onClick={() => setConfirmUninstall(false)}>Cancel</button>
              <button className="settings-btn-danger" onClick={uninstall}>{info.dev ? "Unlink" : "Uninstall"}</button>
            </>
          }
        >
          {info.dev ? (
            <>Removes the dev link and the extension's stored data (key-value storage, launch
              history). Your working directory is not touched.</>
          ) : (
            <>Removes the extension and permanently deletes its stored data (key-value storage,
              launch history).</>
          )}
        </Modal>
      )}
    </div>
  );
}
