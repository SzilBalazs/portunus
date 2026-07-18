import { useState, useEffect, useCallback, useRef, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTauriListener } from "./hooks/useTauriListener";
import { Config, ContentDirEntry } from "./types";
import GeneralSection from "./components/settings/GeneralSection";
import ProvidersSection from "./components/settings/ProvidersSection";
import ClipboardSection from "./components/settings/ClipboardSection";
import DictSection from "./components/settings/DictSection";
import FilesSection from "./components/settings/FilesSection";
import RankingSection from "./components/settings/RankingSection";
import ContentSection from "./components/settings/ContentSection";
import DebugSection from "./components/settings/DebugSection";
import AppearanceSection from "./components/settings/AppearanceSection";
import ExtensionsSection from "./components/settings/ExtensionsSection";
import KeybindsSection from "./components/settings/keybinds/KeybindsSection";
import { DepsProvider } from "./components/settings/DepsContext";
import { applyTheme } from "./theme";
import "./settings.css";
import "./themes.css";

type Section = "general" | "keybinds" | "providers" | "clipboard" | "extensions" | "dict" | "files" | "ranking" | "content" | "debug" | "appearance";

interface NavItem {
  id: Section;
  label: string;
  icon: ReactNode;
}

const NAV: NavItem[] = [
  {
    id: "general",
    label: "General",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <rect x="3" y="3" width="7" height="7"/><rect x="14" y="3" width="7" height="7"/>
        <rect x="14" y="14" width="7" height="7"/><rect x="3" y="14" width="7" height="7"/>
      </svg>
    ),
  },
  {
    id: "keybinds",
    label: "Keybinds",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <rect x="2" y="6" width="20" height="12" rx="2"/>
        <path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/>
      </svg>
    ),
  },
  {
    id: "providers",
    label: "Providers",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
      </svg>
    ),
  },
  {
    id: "clipboard",
    label: "Clipboard",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/>
        <rect x="8" y="2" width="8" height="4" rx="1" ry="1"/>
      </svg>
    ),
  },
  {
    id: "extensions",
    label: "Extensions",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M20.5 11H19V7a2 2 0 0 0-2-2h-4V3.5a2.5 2.5 0 0 0-5 0V5H4a2 2 0 0 0-2 2v3.8h1.5a2.7 2.7 0 0 1 0 5.4H2V20a2 2 0 0 0 2 2h3.8v-1.5a2.7 2.7 0 0 1 5.4 0V22H17a2 2 0 0 0 2-2v-4h1.5a2.5 2.5 0 0 0 0-5z"/>
      </svg>
    ),
  },
  {
    id: "dict",
    label: "Dictionary",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/>
      </svg>
    ),
  },
  {
    id: "files",
    label: "Files",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
      </svg>
    ),
  },
  {
    id: "ranking",
    label: "Ranking",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>
      </svg>
    ),
  },
  {
    id: "content",
    label: "Content",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
        <polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/>
      </svg>
    ),
  },
  {
    id: "appearance",
    label: "Appearance",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="4"/>
        <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41"/>
      </svg>
    ),
  },
  {
    id: "debug",
    label: "Debug",
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>
      </svg>
    ),
  },
];

// Sidebar grouping: keeps all sections but clusters them under labeled dividers
// so the nav scans as intent ("what do I want to change?") instead of a flat list.
// A blank label renders no divider (the top group).
const NAV_GROUPS: { label: string; ids: Section[] }[] = [
  { label: "",               ids: ["general", "keybinds"] },
  { label: "Search sources", ids: ["providers", "files", "content", "clipboard", "dict", "extensions"] },
  { label: "Ranking",        ids: ["ranking"] },
  { label: "Appearance",     ids: ["appearance"] },
  { label: "Advanced",       ids: ["debug"] },
];

const AUTOSAVE_DELAY_MS = 800;

// Drops blank-path directory rows (a just-added row the user hasn't filled in).
// Used both for staging comparisons and before persisting, so an empty row never
// keeps the Apply strip stuck on or lands a junk entry in config.toml.
const nonBlankDirs = (dirs: ContentDirEntry[]) => dirs.filter(d => d.path.trim() !== "");

// "Heavy" content changes need a reindex to take effect, so they are never
// auto-saved. Instead they are staged: the Content section shows an Apply/Revert
// strip, and only "Apply & Reindex" persists them + kicks off the rebuild.
//   - directories / per-dir depth / per-dir extension overrides
//   - the global indexed-extension list
//   - any OCR setting (re-runs extraction over every file)
//   - first-time enable when the index is empty (initial build from scratch)
//   - raising the max file size (previously-skipped large files now need indexing)
// `indexEmpty` reflects whether the on-disk index currently has zero documents.
//
// Single source of truth for the "always heavy" content fields: each has a
// change test, and on a cheap autosave the field is reset to its last-applied
// (base) value. Keeping the set here means `contentHeavyPending` and `stripHeavy`
// can't silently drift. The two *conditional* triggers - first-time enable and a
// max-size increase - are asymmetric, so the callers handle them explicitly.
type ContentField = keyof Config["content"];
const HEAVY_CONTENT_FIELDS: { key: ContentField; changed: (c: Config["content"], b: Config["content"]) => boolean }[] = [
  { key: "dirs", changed: (c, b) => JSON.stringify(nonBlankDirs(c.dirs)) !== JSON.stringify(nonBlankDirs(b.dirs)) },
  { key: "extensions", changed: (c, b) => JSON.stringify(c.extensions) !== JSON.stringify(b.extensions) },
  { key: "ocr_images", changed: (c, b) => c.ocr_images !== b.ocr_images },
  { key: "ocr_pdf_fallback", changed: (c, b) => c.ocr_pdf_fallback !== b.ocr_pdf_fallback },
  { key: "ocr_language", changed: (c, b) => c.ocr_language !== b.ocr_language },
  // Heavy: populating/clearing cached OCR boxes re-OCRs every image (full reindex).
  // Note `ocr_highlight` (the preview-only master toggle) is deliberately NOT here.
  { key: "ocr_highlight_cache", changed: (c, b) => c.ocr_highlight_cache !== b.ocr_highlight_cache },
];

function contentHeavyPending(cur: Config, base: Config, indexEmpty: boolean): boolean {
  const c = cur.content, b = base.content;
  return (
    HEAVY_CONTENT_FIELDS.some(f => f.changed(c, b)) ||
    (c.enabled && !b.enabled && indexEmpty) ||
    c.max_file_bytes > b.max_file_bytes
  );
}

// Returns `cur` with its staged content fields forced back to `base`, so an
// autosave persists the truly cheap edits (e.g. thread count, a max-size
// decrease) while leaving every reindex-triggering edit untouched on disk until
// the user applies them via "Apply & Reindex".
function stripHeavy(cur: Config, base: Config, indexEmpty: boolean): Config {
  const content = { ...cur.content };
  for (const { key } of HEAVY_CONTENT_FIELDS) {
    (content as Record<ContentField, unknown>)[key] = base.content[key];
  }
  if (cur.content.enabled && !base.content.enabled && indexEmpty) {
    content.enabled = base.content.enabled;
  }
  if (cur.content.max_file_bytes > base.content.max_file_bytes) {
    content.max_file_bytes = base.content.max_file_bytes;
  }
  return { ...cur, content };
}

export default function Settings() {
  const [config, setConfig] = useState<Config | null>(null);
  const [activeSection, setActiveSection] = useState<Section>("general");
  const [saving, setSaving] = useState(false);
  const [savedFlash, setSavedFlash] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Set when "Apply & Reindex" fails, shown inline in the Content strip with a retry.
  const [reindexError, setReindexError] = useState<string | null>(null);
  // Set when the backend reports the on-disk config failed to parse (and was reset).
  const [configError, setConfigError] = useState<string | null>(null);
  // Whether the content index is currently empty (drives first-enable detection).
  const [indexEmpty, setIndexEmpty] = useState(true);
  // Live reindex progress, mirrored from the backend so the user gets feedback
  // here even though the launcher (which also shows it) is hidden.
  const [reindexProgress, setReindexProgress] = useState<{ indexed: number; total: number } | null>(null);

  // Tracks the config object reference as it came from disk.
  // Reference equality check lets us skip the auto-save on initial load.
  const diskConfigRef = useRef<Config | null>(null);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const flashTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // True while there are unsaved changes - blocks focus-triggered config reload
  // so a GTK native popup (e.g. select) losing/regaining window focus can't
  // overwrite a just-made change before the autosave timer fires.
  const hasPendingRef = useRef(false);

  const loadConfig = useCallback(async () => {
    try {
      const cfg = await invoke<Config>("get_config");
      diskConfigRef.current = cfg;
      setConfig(cfg);
      applyTheme(cfg.appearance);
      setError(null);
      invoke<boolean>("is_content_index_empty").then(setIndexEmpty).catch(() => {});
      // Surface a one-time warning if the backend had to fall back to defaults
      // because the on-disk config couldn't be parsed.
      invoke<string | null>("take_config_error").then(msg => {
        if (msg) setConfigError(msg);
      }).catch(() => {});
    } catch (e) {
      setError(String(e));
    }
  }, []);

  // Staged heavy content edits awaiting "Apply & Reindex". Derived each render;
  // mirrored into a ref so the focus handler can read it without a stale closure.
  const pendingReindex =
    !!config && !!diskConfigRef.current &&
    contentHeavyPending(config, diskConfigRef.current, indexEmpty);
  const pendingRef = useRef(false);
  useEffect(() => { pendingRef.current = pendingReindex; }, [pendingReindex]);

  // Load config whenever the window gains focus (i.e. every time the user opens it).
  // Debounced because show() + set_focus() each fire the event in quick succession.
  useEffect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    let lastLoad = 0;
    win.onFocusChanged(({ payload: focused }) => {
      if (!focused) return;
      // Don't reload over staged heavy edits - that would silently discard them.
      if (pendingRef.current) return;
      // Don't reload while there are unsaved light changes - a GTK native widget
      // (e.g. a <select> popup) losing then regaining window focus would otherwise
      // overwrite the just-made change before the autosave timer fires.
      if (hasPendingRef.current) return;
      const now = Date.now();
      if (now - lastLoad < 300) return;
      lastLoad = now;
      loadConfig();
    }).then(fn => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, [loadConfig]);

  // Mirror reindex progress from the backend (broadcast to all windows).
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    let doneTimer: ReturnType<typeof setTimeout> | undefined;
    listen<{ indexed: number; total: number }>("content-index-progress", event => {
      const p = event.payload;
      setReindexProgress(p);
      clearTimeout(doneTimer);
      if (p.indexed >= p.total && p.total > 0) {
        doneTimer = setTimeout(() => setReindexProgress(null), 400);
      }
    }).then(fn => { if (active) unlisten = fn; else fn(); });
    return () => { active = false; clearTimeout(doneTimer); unlisten?.(); };
  }, []);


  // Escape closes the window
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") getCurrentWindow().hide();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Jump to a specific section when the launcher opens settings with a target.
  useTauriListener<string>("navigate-to-section", payload => {
    setActiveSection(payload as Section);
  });

  // Marketplace update count for the Extensions nav badge.
  const [updateCount, setUpdateCount] = useState(0);
  const fetchUpdateCount = useCallback(() => {
    invoke<unknown[]>("marketplace_updates")
      .then(u => setUpdateCount(u.length))
      .catch(() => {});
  }, []);
  useEffect(fetchUpdateCount, [fetchUpdateCount]);
  useTauriListener("extensions-reloaded", fetchUpdateCount, [fetchUpdateCount]);
  useTauriListener("marketplace-index-updated", fetchUpdateCount, [fetchUpdateCount]);

  // Apply theme immediately on any appearance change.
  // Only broadcast to main window when it's a user-driven change, not the initial disk load.
  useEffect(() => {
    if (!config) return;
    applyTheme(config.appearance);
    if (diskConfigRef.current && config.appearance !== diskConfigRef.current.appearance) {
      emit("appearance-changed", config.appearance);
    }
  }, [config?.appearance]);

  // Broadcast keybind edits to the launcher immediately (the config watcher
  // re-emits after the autosave lands on disk; the payload event makes the
  // remap feel instant). Reference check skips the initial disk load.
  useEffect(() => {
    if (!config?.keybinds) return;
    if (diskConfigRef.current && config.keybinds !== diskConfigRef.current.keybinds) {
      emit("keybinds-changed", config.keybinds);
    }
  }, [config?.keybinds]);

  // Auto-save: fires 800ms after the last config change, skips on initial load.
  // Heavy content edits are stripped out so only cheap changes hit disk; the
  // heavy ones stay staged until the user clicks "Apply & Reindex".
  useEffect(() => {
    if (!config || config === diskConfigRef.current) return;
    const base = diskConfigRef.current;
    if (!base) return;

    hasPendingRef.current = true;
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    saveTimerRef.current = setTimeout(async () => {
      const toSave = stripHeavy(config, base, indexEmpty);
      // Nothing cheap actually changed (only heavy edits are pending) - don't
      // touch disk, but leave the staged change visible via the Apply strip.
      if (JSON.stringify(toSave) === JSON.stringify(base)) {
        hasPendingRef.current = false;
        return;
      }

      setSaving(true);
      setError(null);
      try {
        await invoke("save_config", { config: toSave });
        diskConfigRef.current = toSave;
        hasPendingRef.current = false;
        setSavedFlash(true);
        if (flashTimerRef.current) clearTimeout(flashTimerRef.current);
        flashTimerRef.current = setTimeout(() => setSavedFlash(false), 1800);
      } catch (e) {
        setError(String(e));
      } finally {
        setSaving(false);
      }
    }, AUTOSAVE_DELAY_MS);

    return () => {
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    };
  }, [config, indexEmpty]);

  // "Apply & Reindex": persist the staged change in full, then trigger the
  // backend rebuild. Progress streams back via content-index-progress.
  //
  // `full` decides cache reuse: only OCR-setting changes invalidate already-cached
  // text and need a clearing rebuild. Dir/extension/depth/size edits run an
  // incremental pass that keeps the existing index (mtime-skip + remove_stale).
  const applyReindex = useCallback(async () => {
    if (!config) return;
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    setSaving(true);
    setError(null);
    setReindexError(null);
    const base = diskConfigRef.current;
    const full =
      !base ||
      config.content.ocr_images !== base.content.ocr_images ||
      config.content.ocr_pdf_fallback !== base.content.ocr_pdf_fallback ||
      config.content.ocr_language !== base.content.ocr_language ||
      // Toggling the box cache (on or off) must re-OCR / clear boxes, so it needs a
      // clearing rebuild — an incremental pass would skip the unchanged images.
      config.content.ocr_highlight_cache !== base.content.ocr_highlight_cache;
    // Strip blank, half-typed rows so they never reach disk.
    const toSave: Config = {
      ...config,
      content: { ...config.content, dirs: nonBlankDirs(config.content.dirs) },
    };
    try {
      await invoke("save_config", { config: toSave });
      diskConfigRef.current = toSave;
      setConfig(toSave);
      await invoke("trigger_full_reindex", { full });
      // A rebuild populates the index, so a subsequent enable is no longer "first".
      setIndexEmpty(false);
      setSavedFlash(true);
      if (flashTimerRef.current) clearTimeout(flashTimerRef.current);
      flashTimerRef.current = setTimeout(() => setSavedFlash(false), 1800);
    } catch (e) {
      // Keep the staged edits intact so the user can fix the cause and retry.
      setReindexError(String(e));
    } finally {
      setSaving(false);
    }
  }, [config]);

  // "Revert": discard the staged edits, restoring the last-applied values.
  const revertReindex = useCallback(() => {
    const base = diskConfigRef.current;
    if (!base || !config) return;
    setReindexError(null);
    setConfig({
      ...config,
      content: {
        ...config.content,
        dirs: base.content.dirs,
        extensions: base.content.extensions,
        ocr_images: base.content.ocr_images,
        ocr_pdf_fallback: base.content.ocr_pdf_fallback,
        ocr_language: base.content.ocr_language,
        ocr_highlight_cache: base.content.ocr_highlight_cache,
        enabled: base.content.enabled,
        max_file_bytes: base.content.max_file_bytes,
      },
    });
  }, [config]);

  const handleClose = () => getCurrentWindow().hide();

  const activeNav = NAV.find(n => n.id === activeSection)!;

  return (
    <div className="settings-window">
      <div className="settings-card">
        {/* Title bar */}
        <div className="settings-titlebar" data-tauri-drag-region>
          <div className="settings-titlebar-left">
            <span className="settings-brand-icon">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="3"/>
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06-.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
              </svg>
            </span>
            <span className="settings-brand-text">Portunus</span>
            <span className="settings-brand-sep">·</span>
            <span className="settings-section-title">{activeNav.label}</span>
          </div>
          <button className="settings-close-btn" onClick={handleClose} title="Close (Esc)">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
              <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
            </svg>
          </button>
        </div>

        {/* Body */}
        <div className="settings-body">
          {/* Sidebar */}
          <div className="settings-sidebar" role="tablist" aria-orientation="vertical">
            {NAV_GROUPS.map(group => (
              <div className="settings-nav-group" key={group.label || "top"}>
                {group.label && <div className="settings-nav-group-label">{group.label}</div>}
                {group.ids.map(id => {
                  const item = NAV.find(n => n.id === id)!;
                  return (
                    <button
                      key={item.id}
                      role="tab"
                      aria-selected={activeSection === item.id}
                      id={`settings-tab-${item.id}`}
                      aria-controls="settings-tabpanel"
                      className={`settings-nav-item${activeSection === item.id ? " active" : ""}`}
                      onClick={() => setActiveSection(item.id)}
                    >
                      <span className="settings-nav-icon">{item.icon}</span>
                      {item.label}
                      {item.id === "extensions" && updateCount > 0 && (
                        <span className="settings-nav-badge">{updateCount}</span>
                      )}
                    </button>
                  );
                })}
              </div>
            ))}
          </div>

          {/* Content */}
          <div
            className="settings-content"
            role="tabpanel"
            id="settings-tabpanel"
            aria-labelledby={`settings-tab-${activeSection}`}
          >
            {configError && (
              <div className="settings-config-error">
                <span className="settings-config-error-msg">
                  Your config file couldn’t be parsed, so defaults are in use:{"\n"}{configError}
                </span>
                <button className="settings-config-error-dismiss" onClick={() => setConfigError(null)}>Dismiss</button>
              </div>
            )}
            {!config ? (
              <div style={{ padding: "24px 0", color: "var(--fg-dim)", fontSize: 13 }}>
                {error ? `Error: ${error}` : "Loading…"}
              </div>
            ) : (
              <DepsProvider>
                <div className={`settings-section-wrap${activeSection === "ranking" ? " settings-section-wrap--wide" : ""}`}>
                {activeSection === "general"   && <GeneralSection   config={config} onChange={setConfig} />}
                {activeSection === "keybinds"  && <KeybindsSection  config={config} onChange={setConfig} />}
                {activeSection === "providers" && <ProvidersSection config={config} onChange={setConfig} />}
                {activeSection === "clipboard" && <ClipboardSection config={config} onChange={setConfig} />}
                {activeSection === "extensions" && <ExtensionsSection config={config} onChange={setConfig} />}
                {activeSection === "dict"      && <DictSection      config={config} onChange={setConfig} />}
                {activeSection === "files"     && <FilesSection     config={config} onChange={setConfig} />}
                {activeSection === "ranking"   && <RankingSection   config={config} onChange={setConfig} />}
                {activeSection === "content"   && (
                  <ContentSection
                    config={config}
                    onChange={setConfig}
                    pendingReindex={pendingReindex}
                    reindexProgress={reindexProgress}
                    reindexError={reindexError}
                    onApply={applyReindex}
                    onRevert={revertReindex}
                  />
                )}
                {activeSection === "debug"      && <DebugSection      config={config} onChange={setConfig} />}
                {activeSection === "appearance" && <AppearanceSection config={config} onChange={setConfig} />}
                </div>
              </DepsProvider>
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="settings-footer">
          <span className="settings-footer-status">
            {error
              ? <span style={{ color: "var(--danger-fg)" }}>Error: {error}</span>
              : saving
                ? <span style={{ color: "var(--fg-mute)" }}>Saving…</span>
                : <span className={`settings-save-status${savedFlash ? " visible" : ""}`}>✓ Saved</span>
            }
          </span>
          <button className="btn-settings-save" onClick={handleClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
