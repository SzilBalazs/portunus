import type { ExtensionPermissions } from "../../../types";
import { NetIcon, StoreIcon, LinkIcon, ClipIcon, PasteIcon, SpawnIcon, BusIcon, KeyIcon, ClockIcon } from "./permGlyphs";

function fmtInterval(secs: number): string {
  if (secs % 3600 === 0) return `${secs / 3600}h`;
  if (secs % 60 === 0) return `${secs / 60}m`;
  return `${secs}s`;
}

interface Row {
  icon: React.ReactNode;
  label: string;
  detail: string;
  danger?: boolean;
  /** Named values shown as wrapping keycap chips below the row (e.g. hosts). */
  chips?: string[];
}

interface Props {
  permissions: ExtensionPermissions | null;
  backgroundIntervalSecs?: number | null;
}

/**
 * Full permission read for one extension — THE consent surface once installed.
 * A danger banner leads when any grant relaxes the sandbox (any-host network,
 * spawned programs, a companion-process channel); each grant is then a row.
 */
export default function PermissionsTab({ permissions: p, backgroundIntervalSecs }: Props) {
  if (!p) return <div className="settings-ext-tab-empty">Permissions unavailable — the extension failed to load.</div>;

  const netAny = p.network.includes("*");
  const hosts = p.network.filter(h => h !== "*");
  const spawn = p.spawn ?? [];

  const rows: Row[] = [];
  if (netAny) rows.push({ icon: <NetIcon />, label: "Network", detail: "any host", danger: true });
  else if (hosts.length) rows.push({ icon: <NetIcon />, label: "Network", detail: `${hosts.length} host${hosts.length > 1 ? "s" : ""}`, chips: hosts });
  if (p.kv) rows.push({ icon: <StoreIcon />, label: "Storage", detail: "local key-value" });
  if (p.open_url) rows.push({ icon: <LinkIcon />, label: "Open URLs", detail: "launch links" });
  if (p.clipboard) rows.push({ icon: <ClipIcon />, label: "Clipboard", detail: "read & write" });
  if (p.paste) rows.push({ icon: <PasteIcon />, label: "Paste", detail: "types into the focused app" });
  if (spawn.length) rows.push({ icon: <SpawnIcon />, label: "Runs programs", detail: spawn.join(", "), danger: true });
  if (p.bus) rows.push({ icon: <BusIcon />, label: "Companion process", detail: "channel to a separately-installed app", danger: true });
  if (p.has_secrets) rows.push({ icon: <KeyIcon />, label: "Secrets", detail: "stored in the system keyring" });
  if (backgroundIntervalSecs != null) rows.push({ icon: <ClockIcon />, label: "Background refresh", detail: `every ${fmtInterval(backgroundIntervalSecs)}` });

  const dangers: string[] = [];
  if (netAny) dangers.push("reach any host on the network");
  if (spawn.length) dangers.push("run programs on your machine");
  if (p.bus) dangers.push("talk to a companion app outside the sandbox");

  return (
    <div className="settings-ext-perm-tab">
      {dangers.length > 0 && (
        <div className="settings-ext-perm-banner">
          <BusIcon />
          <div>
            <b>Relaxes the sandbox.</b> This extension can {dangers.join(", ")}.
          </div>
        </div>
      )}
      {rows.length === 0 ? (
        <div className="settings-ext-tab-empty">No permissions requested.</div>
      ) : (
        <div className="settings-ext-perm-rows">
          {rows.map(r => (
            <div key={r.label} className={`settings-ext-perm-row${r.danger ? " danger" : ""}`}>
              <span className="settings-ext-perm-ico">{r.icon}</span>
              <div className="settings-ext-perm-body">
                <div className="settings-ext-perm-line">
                  <span className="settings-ext-perm-name">{r.label}</span>
                  <span className="settings-ext-perm-detail">{r.detail}</span>
                </div>
                {r.chips && r.chips.length > 0 && (
                  <div className="settings-ext-perm-chips">
                    {r.chips.map(c => <code key={c}>{c}</code>)}
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
