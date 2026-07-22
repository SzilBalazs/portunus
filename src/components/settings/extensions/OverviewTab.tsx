import type { ExtensionInfo, ExtensionPermissions, MarketplaceUpdateInfo } from "../../../types";
import Badge from "../Badge";

/** Short, glanceable summary of what an extension may touch. Danger grants
 * (any host, spawn, companion) surface as plain words so the row stays honest
 * without duplicating the full Permissions tab. */
function permSummary(p: ExtensionPermissions | null): string {
  if (!p) return "unavailable";
  const out: string[] = [];
  const hosts = p.network.filter(h => h !== "*");
  if (p.network.includes("*")) out.push("any host");
  else if (hosts.length) out.push(`${hosts.length} network host${hosts.length > 1 ? "s" : ""}`);
  if (p.kv) out.push("storage");
  if (p.open_url) out.push("open urls");
  if (p.clipboard) out.push("clipboard");
  if (p.paste) out.push("paste");
  if (p.spawn?.length) out.push("runs programs");
  if (p.bus) out.push("companion");
  if (p.has_secrets) out.push("secrets");
  return out.length ? out.join(" · ") : "none";
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="settings-ext-ov-row">
      <span className="settings-ext-ov-label">{label}</span>
      <span className="settings-ext-ov-value">{children}</span>
    </div>
  );
}

interface Props {
  info: ExtensionInfo;
  update?: MarketplaceUpdateInfo;
}

/** Landing tab: full description plus a compact key/value read of the
 * extension's shape (commands, permissions, version, source). */
export default function OverviewTab({ info, update }: Props) {
  const cmds = info.commands.length;
  const origin = info.dev
    ? <>Linked working directory (<code>portunus ext dev</code>)</>
    : info.origin === "marketplace"
      ? <>Marketplace</>
      : info.origin === "url"
        ? <>Legacy URL install — reinstall from the marketplace for updates</>
        : <>Installed locally</>;

  return (
    <div className="settings-ext-overview">
      {info.description && <p className="settings-ext-ov-desc">{info.description}</p>}
      <div className="settings-ext-ov-rows">
        <Row label="Commands">{cmds === 0 ? "None" : `${cmds} command${cmds > 1 ? "s" : ""}`}</Row>
        <Row label="Permissions">{permSummary(info.permissions)}</Row>
        <Row label="Version">
          <span className="settings-ext-ov-inline">
            v{info.version}
            {update && !info.dev
              ? <Badge tone="update">v{update.index_version} available</Badge>
              : info.origin === "marketplace" && !info.dev
                ? <Badge tone="success">Up to date</Badge>
                : null}
          </span>
        </Row>
        <Row label="Source">
          <span className="settings-ext-ov-inline">
            {origin}
            {info.homepage && <a href={info.homepage} target="_blank" rel="noreferrer">{info.homepage}</a>}
          </span>
        </Row>
      </div>
    </div>
  );
}
