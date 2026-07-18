import { useEffect, useState, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { registerProvider, type LaunchContext, type PreviewProps } from "./registry";
import { useTauriListener } from "../hooks/useTauriListener";
import { formatBytes } from "../utils";
import { interpretersIn } from "../spawn";
import type { MarketplaceResult } from "../types";

// Launch routing + preview for marketplace catalog rows. The preview panel is
// the consent surface: it lists every permission the index entry declares, so
// Enter installs a fresh, non-sandbox-breaking extension immediately. Two cases
// arm a confirming second Enter instead of installing outright: a fresh install
// of a `spawn` (sandbox-breaking) extension, and ANY update that grows the
// consented permission set - so a new network host / secret / paste / spawn
// grant gets a deliberate confirmation, not just a passive label.

// ── module state (shared by the launch handler and the preview) ──────────────
const installing = new Set<string>();
let armed: string | null = null;
let stamp = 0;
const listeners = new Set<() => void>();
const bump = () => {
  stamp++;
  listeners.forEach(l => l());
};
const subscribe = (cb: () => void) => {
  listeners.add(cb);
  return () => void listeners.delete(cb);
};
const useMarketStamp = () => useSyncExternalStore(subscribe, () => stamp);

/** Enter (or the action panel's Install row) lands here. */
function startInstall(m: MarketplaceResult, ctx: LaunchContext) {
  if (installing.has(m.name)) return;
  // Arm a confirming second Enter for a fresh sandbox-breaking install, or any
  // update that grows the consented permissions (spawn or otherwise).
  const freshSpawn = m.state === "not_installed" && m.permissions.spawn.length > 0;
  const growsOnUpdate = m.state === "update" && m.permissions_grew;
  const needsConfirm = freshSpawn || growsOnUpdate;
  if (needsConfirm && armed !== m.name) {
    armed = m.name;
    bump();
    return;
  }
  const wasUpdate = m.state === "update";
  installing.add(m.name);
  armed = null;
  bump();
  invoke("marketplace_install", { name: m.name })
    .then(() => {
      ctx.pushToast(`${m.name} ${wasUpdate ? "updated" : "installed"} — ready to use`, "success");
      ctx.requery();
    })
    .catch(e => ctx.pushToast(String(e), "error"))
    .finally(() => {
      installing.delete(m.name);
      bump();
    });
}

registerProvider({
  kinds: ["marketplace", "marketplace-msg"],
  Preview: MarketplacePreview,
  bindableActions: [
    { id: "market:uninstall", title: "Uninstall Extension", hint: "Marketplace" },
  ],
  handleLaunch: (result, ctx) => {
    if (result.kind === "marketplace-msg") return true; // inert info row
    if (result.kind !== "marketplace" || !result.market) return false;
    const m = result.market;
    if (installing.has(m.name)) return true;
    if (m.dev_conflict) {
      ctx.pushToast(`${m.name} is dev-linked — unlink it before installing`, "info");
      return true;
    }
    if (m.state === "installed") {
      ctx.pushToast(`${m.name} is already installed`, "info");
      return true;
    }
    if (m.state === "incompatible") {
      ctx.pushToast(`${m.name} requires a newer Portunus`, "info");
      return true;
    }
    startInstall(m, ctx);
    return true;
  },

  actions: (result, ctx) => {
    if (result.kind !== "marketplace" || !result.market) return [];
    const m = result.market;
    const out = [];
    if (!m.dev_conflict && (m.state === "not_installed" || m.state === "update")) {
      out.push({
        id: "market:install",
        title: m.state === "update" ? `Update to v${m.version}` : "Install",
        section: "result" as const,
        shortcut: { key: "enter" },
        displayOnly: true, // plain Enter already routes through handleLaunch
        run: () => startInstall(m, ctx),
      });
    }
    if (!m.dev_conflict && (m.state === "installed" || m.state === "update")) {
      out.push({
        id: "market:uninstall",
        title: "Uninstall",
        section: "result" as const,
        run: () => {
          invoke("uninstall_extension", { name: m.name })
            .then(() => {
              ctx.pushToast(`${m.name} uninstalled`, "success");
              ctx.requery();
            })
            .catch(e => ctx.pushToast(String(e), "error"));
        },
      });
    }
    return out;
  },
});

// ── preview panel ─────────────────────────────────────────────────────────────

const STATE_BADGE: Record<string, { label: string; tone: string }> = {
  installed: { label: "Installed", tone: "ok" },
  update: { label: "Update available", tone: "update" },
  incompatible: { label: "Needs newer Portunus", tone: "warn" },
};

function permissionLines(m: MarketplaceResult): string[] {
  const p = m.permissions;
  const lines: string[] = [];
  if (p.network.length > 0) lines.push(`Network access: ${p.network.join(", ")}`);
  if (p.clipboard) lines.push("Read the clipboard");
  if (p.kv) lines.push("Store data locally");
  if (p.open_url) lines.push("Open links in your browser");
  if (p.paste) lines.push("Paste into other applications");
  if (p.bus) lines.push("⚠ Talks to a companion app outside the sandbox");
  if (p.has_secrets) lines.push("Store secrets in the system keyring");
  return lines;
}

function MarketplacePreview({ result }: PreviewProps) {
  useMarketStamp();
  if (result.kind === "marketplace-msg") {
    return (
      <div className="market-preview">
        <div className="market-empty">
          <div className="market-empty-title">{result.title}</div>
          <div className="market-empty-desc">{result.subtitle}</div>
        </div>
      </div>
    );
  }
  const m = result.market;
  if (!m) return null;
  return <EntryView m={m} />;
}

function EntryView({ m }: { m: MarketplaceResult }) {
  // Disarm when the selection leaves this row or the marketplace scope, so a
  // stale arm can't turn a later single Enter into an unconfirmed install.
  useEffect(
    () => () => {
      if (armed === m.name) {
        armed = null;
        bump();
      }
    },
    [m.name],
  );

  const busy = installing.has(m.name);
  const isArmed = armed === m.name;
  const badge = STATE_BADGE[m.state];
  const lines = permissionLines(m);
  const spawn = m.permissions.spawn;
  const interpreters = interpretersIn(spawn);

  let hint: string | null;
  if (busy) hint = null;
  else if (m.dev_conflict) hint = "Dev-linked — unlink to install from the marketplace";
  else if (isArmed) hint = "Press Enter again to confirm";
  else if (m.state === "not_installed") hint = "Press Enter to install";
  else if (m.state === "update") hint = "Press Enter to update";
  else hint = null;

  return (
    <div className="market-preview">
      <div className="market-header">
        <div className="market-title-row">
          <span className="market-name">{m.name}</span>
          {m.state === "update" ? (
            <span className="market-version">
              v{m.installed_version} <span className="market-version-arrow">→</span> v{m.version}
            </span>
          ) : (
            <span className="market-version">v{m.version}</span>
          )}
        </div>
        <div className="market-meta-row">
          {m.author && <span>{m.author}</span>}
          {m.size_bytes > 0 && <span>{formatBytes(m.size_bytes)}</span>}
          {badge && <span className={`market-badge ${badge.tone}`}>{badge.label}</span>}
        </div>
      </div>

      {m.description && <div className="market-desc">{m.description}</div>}

      <div className="market-perms">
        <div className="market-perms-label">
          {m.state === "update" && m.permissions_grew
            ? "Permissions — this update requests new ones"
            : "Permissions"}
        </div>
        {lines.length === 0 && spawn.length === 0 ? (
          <div className="market-perm-line none">No permissions — runs fully sandboxed</div>
        ) : (
          <>
            {lines.map(l => (
              <div className="market-perm-line" key={l}>{l}</div>
            ))}
            {spawn.length > 0 && (
              <div className={`market-spawn${isArmed ? " armed" : ""}`}>
                <div className="market-spawn-title">Runs programs outside the sandbox</div>
                <div className="market-spawn-cmds">{spawn.join(", ")}</div>
                {interpreters.length > 0 && (
                  <div className="market-spawn-escalate">
                    {interpreters.join(", ")}{" "}
                    {interpreters.length === 1
                      ? "is a command interpreter — it can"
                      : "are command interpreters — they can"}{" "}
                    run <strong>any</strong> program, granting effectively unrestricted access.
                  </div>
                )}
              </div>
            )}
          </>
        )}
      </div>

      {busy ? (
        <InstallProgress name={m.name} />
      ) : (
        hint && <div className={`market-hint${isArmed ? " armed" : ""}`}>{hint}</div>
      )}
    </div>
  );
}

function InstallProgress({ name }: { name: string }) {
  // Re-render on every throttled progress event while this install runs.
  const [progress, setProgress] = useState<{ fetched: number; total: number | null } | null>(null);
  useTauriListener<{ fetched: number; total: number | null }>(
    "ext-install-progress",
    p => setProgress(p),
  );
  const pct =
    progress && progress.total ? Math.min(100, (progress.fetched / progress.total) * 100) : null;
  return (
    <div className="market-progress">
      <div className="market-progress-label">
        Installing {name}
        {progress && (
          <span>
            {" "}
            — {formatBytes(progress.fetched)}
            {progress.total ? ` / ${formatBytes(progress.total)}` : ""}
          </span>
        )}
      </div>
      <div className="market-progress-bar">
        <div
          className={`market-progress-fill${pct === null ? " indeterminate" : ""}`}
          style={pct !== null ? { width: `${pct}%` } : undefined}
        />
      </div>
    </div>
  );
}

const STYLES = `
.market-preview {
  padding: 16px 18px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}
.market-preview::-webkit-scrollbar { width: 4px; }
.market-preview::-webkit-scrollbar-thumb { background: var(--bg-input); border-radius: 2px; }
.market-preview::-webkit-scrollbar-track { background: transparent; }

.market-header {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-bottom: 10px;
  border-bottom: 1px solid var(--line);
}
.market-title-row {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}
.market-name {
  font-size: 19px;
  font-weight: 700;
  color: var(--fg);
  letter-spacing: -0.02em;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.market-version {
  font: 400 11px/1 "JetBrains Mono","Fira Code",monospace;
  color: var(--fg-dim);
  white-space: nowrap;
}
.market-version-arrow { color: var(--accent); }
.market-meta-row {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 11px;
  color: var(--fg-mute);
}
.market-badge {
  font-size: 10px;
  padding: 2px 7px;
  border-radius: 99px;
  background: var(--accent-soft);
  color: var(--accent);
}
.market-badge.warn { opacity: 0.75; }

.market-desc {
  font-size: 12.5px;
  line-height: 1.5;
  color: var(--fg-desc);
}

.market-perms {
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.market-perms-label {
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.07em;
  color: var(--fg-mute);
  margin-bottom: 2px;
}
.market-perm-line {
  font-size: 12px;
  color: var(--fg-desc);
  padding-left: 10px;
  position: relative;
}
.market-perm-line::before {
  content: "";
  position: absolute;
  left: 0;
  top: 7px;
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--fg-dim);
}
.market-perm-line.none { color: var(--fg-dim); padding-left: 0; }
.market-perm-line.none::before { display: none; }

.market-spawn {
  margin-top: 4px;
  padding: 8px 10px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--line);
  background: color-mix(in srgb, var(--bg-card) 85%, var(--accent) 15%);
}
.market-spawn.armed { border-color: var(--accent); }
.market-spawn-title {
  font-size: 11px;
  font-weight: 600;
  color: var(--fg);
  margin-bottom: 3px;
}
.market-spawn-cmds {
  font: 400 11px/1.4 "JetBrains Mono","Fira Code",monospace;
  color: var(--fg-desc);
  word-break: break-word;
}
.market-spawn-escalate {
  margin-top: 6px;
  padding-top: 6px;
  border-top: 1px solid var(--line);
  font-size: 11px;
  line-height: 1.4;
  color: var(--fg-desc);
}
.market-spawn-escalate strong { color: var(--fg); }

.market-hint {
  margin-top: auto;
  font-size: 11px;
  color: var(--fg-mute);
  padding-top: 8px;
}
.market-hint.armed { color: var(--accent); }

.market-progress {
  margin-top: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-top: 8px;
}
.market-progress-label { font-size: 11px; color: var(--fg-mute); }
.market-progress-bar {
  height: 3px;
  border-radius: 2px;
  background: var(--bg-input);
  overflow: hidden;
}
.market-progress-fill {
  height: 100%;
  background: var(--accent);
  border-radius: 2px;
  transition: width 0.15s ease;
}
.market-progress-fill.indeterminate {
  width: 40%;
  animation: market-indeterminate 1.1s ease-in-out infinite;
}
@keyframes market-indeterminate {
  0% { margin-left: -40%; }
  100% { margin-left: 100%; }
}

.market-empty {
  margin: auto;
  text-align: center;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.market-empty-title { font-size: 14px; font-weight: 600; color: var(--fg); }
.market-empty-desc { font-size: 12px; color: var(--fg-mute); }
`;

if (typeof document !== "undefined") {
  const id = "market-preview-styles";
  if (!document.getElementById(id)) {
    const el = document.createElement("style");
    el.id = id;
    el.textContent = STYLES;
    document.head.appendChild(el);
  }
}
