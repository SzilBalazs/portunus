import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTauriListener } from "../../hooks/useTauriListener";
import { Config, ExtensionInfo, MarketplaceUpdateInfo } from "../../types";
import { SearchIcon, RefreshIcon } from "../../icons";
import SectionHeader from "./SectionHeader";
import ExtensionCard from "./extensions/ExtensionCard";
import InstallExtensionDialog from "./extensions/InstallExtensionDialog";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
  /** Deep-link: name of an extension to surface on open (from the launcher's
   *  marketplace "Open in Settings"). Pre-fills the filter to that card. */
  focusExtension?: string | null;
}

export default function ExtensionsSection({ config, onChange, focusExtension }: Props) {
  const [exts, setExts] = useState<ExtensionInfo[] | null>(null);
  const [installOpen, setInstallOpen] = useState(false);
  const [storageDegraded, setStorageDegraded] = useState<string | null>(null);
  const [secretsAvailable, setSecretsAvailable] = useState(true);
  const [filter, setFilter] = useState("");

  // Surface the requested card when arriving via a deep-link.
  useEffect(() => {
    if (focusExtension) setFilter(focusExtension);
  }, [focusExtension]);

  useEffect(() => {
    invoke<string | null>("extension_storage_status")
      .then(setStorageDegraded)
      .catch(() => {});
    invoke<boolean>("secrets_available")
      .then(setSecretsAvailable)
      .catch(() => {});
  }, []);

  const refresh = useCallback(() => {
    invoke<ExtensionInfo[]>("list_extensions")
      .then(next =>
        // Skip the state update when nothing changed - a no-op refresh after
        // Rescan would otherwise re-render every row (visible as a flash).
        setExts(prev => prev && JSON.stringify(prev) === JSON.stringify(next) ? prev : next))
      .catch(() => setExts([]));
  }, []);
  useEffect(refresh, [refresh]);
  // Fired by the backend when an extension rebuild completes.
  useTauriListener("search-invalidated", refresh, [refresh]);
  // Installs/uninstalls/reloads emit this dedicated signal.
  useTauriListener("extensions-reloaded", refresh, [refresh]);
  // Runtime errors happen while the user is in the LAUNCHER; refresh on focus.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => { if (focused) refresh(); })
      .then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, [refresh]);

  const setEnabled = (name: string, value: boolean) => {
    const entry = config.extensions[name] ?? { enabled: false, settings: {} };
    onChange({ ...config, extensions: { ...config.extensions, [name]: { ...entry, enabled: value } } });
  };

  const rescan = () => { invoke("rescan_extensions").catch(() => {}); };

  // Available updates from the marketplace index (cheap - no downloads).
  const [updates, setUpdates] = useState<MarketplaceUpdateInfo[]>([]);
  const [checking, setChecking] = useState(false);
  const [checkMsg, setCheckMsg] = useState<string | null>(null);

  const fetchUpdates = useCallback(() => {
    invoke<MarketplaceUpdateInfo[]>("marketplace_updates").then(setUpdates).catch(() => {});
  }, []);
  useEffect(fetchUpdates, [fetchUpdates]);
  useTauriListener("extensions-reloaded", fetchUpdates, [fetchUpdates]);
  useTauriListener("marketplace-index-updated", fetchUpdates, [fetchUpdates]);

  // Force-refreshes the index, then re-reads the update list.
  const checkForUpdates = async () => {
    setChecking(true);
    setCheckMsg(null);
    try {
      await invoke("marketplace_refresh", { force: true });
      const next = await invoke<MarketplaceUpdateInfo[]>("marketplace_updates");
      setUpdates(next);
      setCheckMsg(next.length === 0 ? "Up to date" : `${next.length} update${next.length > 1 ? "s" : ""}`);
    } catch (e) {
      setCheckMsg(String(e));
    }
    setChecking(false);
    window.setTimeout(() => setCheckMsg(null), 4000);
  };

  // Dev-linked extensions sort first: they're the ones being actively worked on.
  const sorted = exts ? [...exts].sort((a, b) => Number(b.dev) - Number(a.dev) || a.name.localeCompare(b.name)) : null;
  const q = filter.trim().toLowerCase();
  const visible = q
    ? sorted?.filter(e => e.name.toLowerCase().includes(q) || (e.description ?? "").toLowerCase().includes(q))
    : sorted;

  return (
    <div className="settings-section">
      <SectionHeader
        title="Extensions"
        desc={<>WASM extensions extend search with new sources and actions. Install from the marketplace (search <em>marketplace</em> in the launcher), from a <code>.portext</code> file, or drop a folder into <code>~/.local/share/portunus/extensions/</code>. Nothing runs until you review its permissions.</>}
      />

      {storageDegraded && (
        <div className="settings-dep-inline-warn">
          Extension storage is running in memory — data will be lost on quit ({storageDegraded})
        </div>
      )}

      {/* One toolbar for the whole section: filter, plus the three global
          actions (check-for-updates / install / rescan) that used to be
          scattered across separate rows. */}
      <div className="settings-ext-toolbar">
        <div className="settings-ext-filter">
          <SearchIcon />
          <input
            value={filter}
            onChange={e => setFilter(e.target.value)}
            placeholder="Filter extensions…"
            spellCheck={false}
          />
        </div>
        <span className="settings-ext-card-spacer" />
        <button className="settings-btn-secondary" onClick={checkForUpdates} disabled={checking} title="Check the marketplace for updates">
          <RefreshIcon />{checking ? "Checking…" : checkMsg ?? "Updates"}
        </button>
        <button className="settings-btn-primary" onClick={() => setInstallOpen(true)} title="Sideload a .portext file — shows permissions and hash first">
          Install…
        </button>
        <button className="settings-btn-secondary" onClick={rescan} title="Re-discover the extensions directory and reload wasm files">
          Rescan
        </button>
      </div>

      {sorted === null && <div className="settings-dep-empty">Scanning…</div>}
      {sorted?.length === 0 && (
        <div className="settings-dep-empty">
          No extensions installed yet. Install one above, or scaffold your own with <code>portunus ext new</code>.
        </div>
      )}
      {sorted && sorted.length > 0 && visible?.length === 0 && (
        <div className="settings-dep-empty">No extensions match “{filter.trim()}”.</div>
      )}

      {visible && visible.length > 0 && (
        <div className="settings-ext-cards">
          {visible.map(info => (
            <ExtensionCard
              key={info.name}
              info={info}
              // Live toggle state comes from config; `info` is a backend snapshot.
              enabled={config.extensions[info.name]?.enabled ?? false}
              isNew={!(info.name in config.extensions)}
              secretsAvailable={secretsAvailable}
              update={updates.find(u => u.name === info.name)}
              onSetEnabled={v => setEnabled(info.name, v)}
              onChanged={refresh}
            />
          ))}
        </div>
      )}

      {installOpen && (
        <InstallExtensionDialog onClose={() => setInstallOpen(false)} onInstalled={refresh} />
      )}
    </div>
  );
}
