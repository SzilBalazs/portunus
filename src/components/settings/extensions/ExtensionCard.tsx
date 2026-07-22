import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ExtensionInfo, MarketplaceUpdateInfo } from "../../../types";
import { WarnIcon } from "../../../icons";
import Toggle from "../Toggle";
import Badge from "../Badge";
import Modal from "../Modal";
import PermissionChips from "./PermissionChips";
import ExtensionSettingsForm from "./ExtensionSettingsForm";
import ExtensionLogs from "./ExtensionLogs";
import DangerConsentModal from "./DangerConsentModal";
import ExtCardTabs, { type ExtTab } from "./ExtCardTabs";
import OverviewTab from "./OverviewTab";
import PermissionsTab from "./PermissionsTab";

interface Props {
  info: ExtensionInfo;
  enabled: boolean;
  /** Not yet present in config - never enabled, still needs review. */
  isNew: boolean;
  /** Whether a Secret Service daemon is reachable (for secret settings). */
  secretsAvailable: boolean;
  /** Available marketplace update for this extension, if any. */
  update?: MarketplaceUpdateInfo;
  onSetEnabled: (v: boolean) => void;
  onChanged: () => void;
}

/**
 * One installed extension: a collapsed header row (name, version, badges,
 * enable toggle) with a status rail down its left edge. Expanding reveals the
 * detail split across in-card tabs - Overview, Commands, Permissions, Settings
 * and Logs - over a persistent action footer (update / uninstall / re-consent).
 */
export default function ExtensionCard({ info, enabled, isNew, secretsAvailable, update, onSetEnabled, onChanged }: Props) {
  const [expanded, setExpanded] = useState(true);
  const [tab, setTab] = useState("overview");
  const [updating, setUpdating] = useState(false);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [confirmUninstall, setConfirmUninstall] = useState(false);
  // Which danger-consent gate is pending, if any. Sandbox-relaxing grants (a
  // spawn allowlist, any-host network) must clear the same hard checkbox the
  // install dialog enforces when enabling, re-approving, or updating into them
  // - not just the passive badge/chips.
  const [dangerConsent, setDangerConsent] = useState<null | "enable" | "reconsent" | "update">(null);
  // A non-danger permission-growth update opens a plain permission-diff review.
  const [permReview, setPermReview] = useState(false);
  const spawnCmds = info.permissions?.spawn ?? [];
  const netAny = (info.permissions?.network ?? []).includes("*");
  const busGrant = info.permissions?.bus ?? false;
  // On update: the incoming version's grants, and whether "*" is newly gained.
  const updNetAny = (update?.permissions.network ?? []).includes("*");
  const netBecameAny = updNetAny && !netAny;
  const busBecameGranted = (update?.permissions.bus ?? false) && !busGrant;

  const doUpdate = () => {
    setUpdating(true);
    setUpdateError(null);
    invoke("marketplace_install", { name: info.name })
      .then(onChanged)
      .catch(e => setUpdateError(String(e)))
      .finally(() => setUpdating(false));
  };

  // A marketplace update re-consents to the new grant set, so growth gets the
  // same review as a fresh install: a grown spawn allowlist or newly-gained
  // any-host network requires the hard danger acknowledgement, any other growth
  // a permission diff. An update that grows nothing installs directly.
  const runUpdate = () => {
    if (!update) return doUpdate();
    const spawnGrew = (update.permissions.spawn ?? []).some(c => !spawnCmds.includes(c));
    if (spawnGrew || netBecameAny || busBecameGranted) { setDangerConsent("update"); return; }
    if (update.permissions_grew) { setPermReview(true); return; }
    doUpdate();
  };

  const uninstall = () => {
    invoke("uninstall_extension", { name: info.name })
      .then(onChanged)
      .catch(e => console.error(`[extensions] uninstall failed: ${e}`));
    setConfirmUninstall(false);
  };

  const doReconsent = () => {
    invoke("consent_extension_permissions", { name: info.name })
      .then(() => { invoke("rescan_extensions").catch(() => {}); onChanged(); })
      .catch(e => console.error(`[extensions] consent failed: ${e}`));
  };

  // Enabling or re-approving a sandbox-relaxing extension (spawn or any-host
  // network) routes through the blocking consent modal first; everything else
  // applies immediately.
  const needsDangerConsent = spawnCmds.length > 0 || netAny || busGrant;
  const handleSetEnabled = (v: boolean) => {
    if (v && needsDangerConsent) { setDangerConsent("enable"); return; }
    onSetEnabled(v);
  };
  const reconsent = () => {
    if (needsDangerConsent) { setDangerConsent("reconsent"); return; }
    doReconsent();
  };

  // No parsed manifest = the extension cannot load; the toggle would be a lie.
  const broken = !info.permissions;
  const hasIssue = Boolean(info.error) || info.benched || broken || info.needs_reconsent;
  // Left status rail: red = needs attention, accent = live & healthy, grey = off.
  const railState = hasIssue ? "issue" : enabled && !broken ? "on" : "off";

  const tabs: ExtTab[] = [{ key: "overview", label: "Overview" }];
  if (info.commands.length > 0) tabs.push({ key: "commands", label: "Commands", count: info.commands.length });
  tabs.push({ key: "permissions", label: "Permissions", alert: needsDangerConsent || info.needs_reconsent });
  if (info.settings_schema.length > 0) tabs.push({ key: "settings", label: "Settings", count: info.settings_schema.length });
  tabs.push({ key: "logs", label: "Logs" });
  // A data change can retire the active tab (e.g. settings schema emptied) -
  // fall back to Overview, which is always present.
  const active = tabs.some(t => t.key === tab) ? tab : "overview";

  return (
    <div className={`settings-ext-card${expanded ? " expanded" : ""}`}>
      <div className="settings-ext-rail" data-state={railState} />
      <div className="settings-ext-card-main">
        <div className="settings-ext-card-header" onClick={() => setExpanded(e => !e)}>
          <span className="settings-ext-card-name">{info.name}</span>
          {info.version && <span className="settings-ext-version">v{info.version}</span>}
          {info.dev && <Badge tone="dev">dev</Badge>}
          {update && !info.dev && (
            <Badge tone="update">
              v{update.index_version} available{update.permissions_grew ? " — new permissions" : ""}
            </Badge>
          )}
          {spawnCmds.length > 0 && <Badge tone="danger">runs programs</Badge>}
          {netAny && <Badge tone="danger">any host</Badge>}
          {busGrant && <Badge tone="danger">companion app</Badge>}
          {isNew && !broken && <Badge tone="new">new — review &amp; enable</Badge>}
          {info.needs_reconsent && <Badge tone="update">permissions changed</Badge>}
          {(info.error || info.benched) && !info.needs_reconsent && <Badge tone="error">error</Badge>}
          <span className="settings-ext-card-spacer" />
          {!broken && (
            <span className="settings-ext-onoff" data-on={enabled}>{enabled ? "ON" : "OFF"}</span>
          )}
          <span onClick={e => e.stopPropagation()} title={broken ? "Fix the error below to enable" : undefined}>
            <Toggle label={info.name} checked={enabled && !broken} disabled={broken} onChange={handleSetEnabled} />
          </span>
          <svg className="settings-ext-chevron" width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <path d="M3.5 2l3 3-3 3" />
          </svg>
        </div>

        {expanded && (
          <>
            <ExtCardTabs tabs={tabs} active={active} onSelect={setTab} />
            <div className="settings-ext-tabpanel">
              {active === "overview" && (
                <>
                  {info.error && <div className="settings-dep-inline-warn"><WarnIcon />{info.error}</div>}
                  {info.benched && <div className="settings-dep-inline-warn"><WarnIcon />Paused after repeated failures — retries automatically; fix and Rescan to retry now.</div>}
                  <OverviewTab info={info} update={update} />
                </>
              )}
              {active === "commands" && (
                <div className="settings-ext-cmd-list">
                  {info.commands.map(c => (
                    <div className="settings-ext-cmd-row" key={c.name}>
                      <span className="settings-ext-cmd-title">
                        {c.title}
                        {c.always && <span className="settings-ext-cmd-always">every keystroke</span>}
                      </span>
                      <span className="settings-ext-cmd-mode">
                        <Badge tone={c.mode === "action" ? "new" : "neutral"}>{c.mode}</Badge>
                      </span>
                      <span className="settings-ext-cmd-triggers">
                        {c.keywords.map(k => <code key={k}>{k}</code>)}
                      </span>
                    </div>
                  ))}
                </div>
              )}
              {active === "permissions" && (
                <PermissionsTab permissions={info.permissions} backgroundIntervalSecs={info.background_interval_secs} />
              )}
              {active === "settings" && (
                <ExtensionSettingsForm extension={info.name} schema={info.settings_schema} values={info.settings_values} secretsSet={info.secrets_set} secretsAvailable={secretsAvailable} onChanged={onChanged} />
              )}
              {active === "logs" && <ExtensionLogs extension={info.name} />}
            </div>

            {updateError && (
              <div className="settings-ext-footer-warn settings-dep-inline-warn"><WarnIcon />{updateError}</div>
            )}
            <div className="settings-ext-footer">
              {info.needs_reconsent && (
                <button className="settings-btn-primary" onClick={reconsent}>Review &amp; allow new permissions</button>
              )}
              {update && !info.dev && (
                <button className="settings-btn-primary" onClick={runUpdate} disabled={updating}>
                  {updating ? "Updating…" : `Update to v${update.index_version}`}
                </button>
              )}
              <span className="settings-ext-card-spacer" />
              <button className="settings-btn-danger" onClick={() => setConfirmUninstall(true)}>
                {info.dev ? "Unlink" : "Uninstall"}
              </button>
            </div>
          </>
        )}
      </div>

      {dangerConsent && (
        <DangerConsentModal
          title={
            dangerConsent === "enable"
              ? `Enable ${info.name}?`
              : dangerConsent === "update"
                ? `Update ${info.name}?`
                : `Allow new permissions for ${info.name}?`
          }
          spawnCommands={dangerConsent === "update" ? update?.permissions.spawn ?? [] : spawnCmds}
          networkAny={dangerConsent === "update" ? netBecameAny : netAny}
          bus={dangerConsent === "update" ? busBecameGranted : busGrant}
          confirmLabel={dangerConsent === "enable" ? "Enable" : dangerConsent === "update" ? "Update" : "Allow"}
          onCancel={() => setDangerConsent(null)}
          onConfirm={() => {
            const which = dangerConsent;
            setDangerConsent(null);
            if (which === "enable") onSetEnabled(true);
            else if (which === "update") doUpdate();
            else doReconsent();
          }}
        />
      )}

      {permReview && update && (
        <Modal
          title={`Update ${info.name} to v${update.index_version}?`}
          onClose={() => setPermReview(false)}
          width={470}
          footer={
            <>
              <button className="settings-btn-secondary" onClick={() => setPermReview(false)}>Cancel</button>
              <button
                className="settings-btn-primary"
                onClick={() => { setPermReview(false); doUpdate(); }}
              >
                Update to v{update.index_version}
              </button>
            </>
          }
        >
          <div className="settings-field-desc">
            This update requests new permissions. Review them before updating.
          </div>
          <PermissionChips permissions={update.permissions} diffAgainst={info.permissions} />
        </Modal>
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
