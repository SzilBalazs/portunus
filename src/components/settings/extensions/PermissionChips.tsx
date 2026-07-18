import type { ExtensionPermissions } from "../../../types";

interface Props {
  permissions: ExtensionPermissions | null;
  backgroundIntervalSecs?: number | null;
  /** Previously-consented snapshot; grants not in it render highlighted. */
  diffAgainst?: ExtensionPermissions | null;
}

function fmtInterval(secs: number): string {
  if (secs % 3600 === 0) return `${secs / 3600}h`;
  if (secs % 60 === 0) return `${secs / 60}m`;
  return `${secs}s`;
}

/**
 * Human summary of what an extension may touch - THE consent surface, shown
 * before enable/install. With `diffAgainst`, newly-requested grants get the
 * `new` styling so a permissions-grew update is reviewable at a glance.
 */
export default function PermissionChips({ permissions, backgroundIntervalSecs, diffAgainst }: Props) {
  if (!permissions) return null;
  const chips: { text: string; grew: boolean; danger?: boolean }[] = [];
  const old = diffAgainst;
  for (const host of permissions.network) {
    // "*" is sandbox-relaxing (any host) - flag it like a danger grant.
    if (host === "*") {
      chips.push({
        text: "⚠ network: any host",
        grew: old ? !old.network.includes("*") : false,
        danger: true,
      });
      continue;
    }
    chips.push({ text: `network: ${host}`, grew: old ? !old.network.includes(host) : false });
  }
  if (permissions.kv) chips.push({ text: "storage", grew: old ? !old.kv : false });
  if (permissions.clipboard) chips.push({ text: "clipboard", grew: old ? !old.clipboard : false });
  if (permissions.open_url) chips.push({ text: "open urls", grew: old ? !old.open_url : false });
  if (permissions.paste) chips.push({ text: "paste keystrokes", grew: old ? !old.paste : false });
  // Sandbox-breaking: flag it as danger and name the exact binaries.
  const spawn = permissions.spawn ?? [];
  if (spawn.length > 0) {
    const oldSpawn = old?.spawn ?? [];
    chips.push({
      text: `⚠ runs programs: ${spawn.join(", ")}`,
      grew: old ? spawn.some(c => !oldSpawn.includes(c)) : false,
      danger: true,
    });
  }
  // The companion process runs outside the sandbox - flag like spawn.
  if (permissions.bus) {
    chips.push({ text: "⚠ companion process channel", grew: old ? !old.bus : false, danger: true });
  }
  if (permissions.has_secrets) chips.push({ text: "secrets (keyring)", grew: old ? !old.has_secrets : false });
  if (chips.length === 0) chips.push({ text: "no permissions", grew: false });
  if (backgroundIntervalSecs != null) {
    chips.push({ text: `background: every ${fmtInterval(backgroundIntervalSecs)}`, grew: false });
  }
  return (
    <div className="settings-ext-perms">
      {chips.map(c => (
        <code
          key={c.text}
          className={c.danger ? "settings-ext-perm-danger" : c.grew ? "settings-ext-perm-new" : undefined}
        >
          {c.grew ? "+ " : ""}{c.text}
        </code>
      ))}
    </div>
  );
}
