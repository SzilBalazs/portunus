import { useEffect, useRef, useState, type ReactElement } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { Config, ContentDirEntry, DepStatus, DeSetupInfo, DesktopEnv } from "../../types";
import Toggle from "../settings/Toggle";
import "./onboarding.css";

interface Props {
  config: Config;
  onComplete: () => void;
}

const STEPS = ["Welcome", "Desktop", "Content"] as const;

// ── feature tour data ────────────────────────────────────────────────────────

const I = (paths: ReactElement) => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">{paths}</svg>
);

interface Feature {
  id: string;
  name: string;
  desc: string;
  example?: string;
  icon: ReactElement;
}

const FEATURES: Feature[] = [
  { id: "apps", name: "Applications", desc: "Launch any installed app", example: "firefox",
    icon: I(<><rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></>) },
  { id: "files", name: "Files", desc: "Find files by name, fuzzy-matched", example: "report.pdf",
    icon: I(<><path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><path d="M13 2v7h7"/></>) },
  { id: "calc", name: "Calculator", desc: "Math, units, currency, dates", example: "5km to mi",
    icon: I(<><rect x="4" y="2" width="16" height="20" rx="2"/><line x1="8" y1="6" x2="16" y2="6"/><line x1="8" y1="11" x2="8" y2="11"/><line x1="12" y1="11" x2="12" y2="11"/><line x1="16" y1="11" x2="16" y2="11"/><line x1="8" y1="15" x2="8" y2="15"/><line x1="12" y1="15" x2="12" y2="15"/><line x1="16" y1="15" x2="16" y2="18"/></>) },
  { id: "dict", name: "Dictionary", desc: "Word definitions", example: "define lucid",
    icon: I(<><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></>) },
  { id: "clipboard", name: "Clipboard", desc: "Search your clipboard history", example: "portunus --clipboard",
    icon: I(<><rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/></>) },
  { id: "content", name: "Contents", desc: "Full-text search inside documents", example: "Tab ⇢ invoice",
    icon: I(<><circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/><path d="M8 11h6"/><path d="M11 8v6"/></>) },
  { id: "extensions", name: "Extensions", desc: "Add new sources with sandboxed extensions", example: "emoji shrug",
    icon: I(<><path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z"/><path d="m3.3 7 8.7 5 8.7-5"/><path d="M12 22V12"/></>) },
];

// ── per-DE setup guides ──────────────────────────────────────────────────────
// The backend reports raw facts (detected DE + resolved exec path); everything
// below is presentation. Hotkeys are never registered by us - the user pastes.

interface HotkeyGuide {
  /** Where the snippet goes: a config file path or a GUI location. */
  where: string;
  text: string;
  /** Extra caution rendered under the snippet (e.g. gsettings list overwrite). */
  caveat?: string;
}

interface DeGuide {
  label: string;
  hotkey: (exec: string) => HotkeyGuide;
  /** Whether this desktop honors ~/.config/autostart (XDG autostart). */
  xdgAutostart: boolean;
  /** Compositor exec line shown instead of the toggle when XDG autostart is ignored. */
  autostart?: (exec: string) => HotkeyGuide;
}

const DE_GUIDES: Record<DesktopEnv, DeGuide> = {
  hyprland: {
    label: "Hyprland",
    hotkey: exec => ({ where: "~/.config/hypr/hyprland.conf", text: `bind = SUPER, SPACE, exec, ${exec} --show` }),
    xdgAutostart: false,
    autostart: exec => ({ where: "~/.config/hypr/hyprland.conf", text: `exec-once = ${exec}` }),
  },
  sway: {
    label: "sway",
    hotkey: exec => ({ where: "~/.config/sway/config", text: `bindsym $mod+space exec ${exec} --show` }),
    xdgAutostart: false,
    autostart: exec => ({ where: "~/.config/sway/config", text: `exec ${exec}` }),
  },
  niri: {
    label: "niri",
    hotkey: exec => ({ where: "~/.config/niri/config.kdl", text: `Mod+Space { spawn "${exec}" "--show"; }` }),
    xdgAutostart: false,
    autostart: exec => ({ where: "~/.config/niri/config.kdl", text: `spawn-at-startup "${exec}"` }),
  },
  river: {
    label: "river",
    hotkey: exec => ({ where: "~/.config/river/init", text: `riverctl map normal Super Space spawn '${exec} --show'` }),
    xdgAutostart: false,
    autostart: exec => ({ where: "~/.config/river/init", text: `${exec} &` }),
  },
  gnome: {
    label: "GNOME",
    hotkey: exec => ({
      where: "Settings → Keyboard → Custom Shortcuts, or a terminal:",
      text: [
        `gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/portunus/']"`,
        `gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/portunus/ name 'Portunus'`,
        `gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/portunus/ command '${exec} --show'`,
        `gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/portunus/ binding '<Super>space'`,
      ].join("\n"),
      caveat: "The first line replaces any custom shortcuts you already have - prefer the Settings GUI if you use them.",
    }),
    xdgAutostart: true,
  },
  kde: {
    label: "KDE Plasma",
    hotkey: exec => ({
      where: "System Settings → Shortcuts → Add New → Command or Script:",
      text: `${exec} --show`,
    }),
    xdgAutostart: true,
  },
  other: {
    label: "your desktop",
    hotkey: exec => ({
      where: "Bind a key in your desktop's keyboard settings to:",
      text: `${exec} --show`,
    }),
    xdgAutostart: true,
  },
};

// ── content presets ──────────────────────────────────────────────────────────

type PresetId = "skip" | "docs" | "ocr";

interface Preset {
  id: PresetId;
  name: string;
  blurb: string;
  dirs: ContentDirEntry[];
  ocr: boolean;
  /** External binary required for this preset to fully work. */
  requires?: string;
}

const DOCS_DIRS: ContentDirEntry[] = [
  { path: "~/Downloads", depth: 2, extensions: ["pdf", "docx"] },
  { path: "~/Documents", depth: 3, extensions: ["pdf", "docx"] },
];

const OCR_DIRS: ContentDirEntry[] = [
  { path: "~/Downloads", depth: 2, extensions: ["pdf", "docx", "png", "jpg", "jpeg"] },
  { path: "~/Documents", depth: 3, extensions: ["pdf", "docx", "png", "jpg", "jpeg"] },
];

const PRESETS: Preset[] = [
  { id: "skip", name: "Skip for now", blurb: "Leave content search off; enable it anytime in Settings → Content.", dirs: [], ocr: false },
  { id: "docs", name: "Documents", blurb: "Index PDFs & Word docs in Downloads and Documents. Fast, text-only.", dirs: DOCS_DIRS, ocr: false },
  { id: "ocr", name: "Documents + OCR", blurb: "Also reads scanned PDFs and images via OCR. Most thorough.", dirs: OCR_DIRS, ocr: true, requires: "tesseract" },
];

// ──────────────────────────────────────────────────────────────────────────────

export default function OnboardingWizard({ config, onComplete }: Props) {
  const [draft, setDraft] = useState<Config>(config);
  const [step, setStep] = useState(0);
  const [dir, setDir] = useState<1 | -1>(1);
  const [version, setVersion] = useState("");
  const [deps, setDeps] = useState<Record<string, boolean> | null>(null);
  const [deInfo, setDeInfo] = useState<DeSetupInfo | null>(null);
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartError, setAutostartError] = useState(false);
  const [preset, setPreset] = useState<PresetId>("skip");
  const [finishing, setFinishing] = useState(false);
  const [saveError, setSaveError] = useState(false);
  const primaryRef = useRef<HTMLButtonElement>(null);
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => { getVersion().then(setVersion); }, []);

  // Probe optional system dependencies once so the content step can warn.
  useEffect(() => {
    invoke<DepStatus[]>("check_dependencies")
      .then(list => setDeps(Object.fromEntries(list.map(d => [d.id, d.available]))))
      .catch(() => setDeps({}));
  }, []);

  // Desktop facts + current autostart state for the Desktop step.
  useEffect(() => {
    invoke<DeSetupInfo>("de_setup_info")
      .then(setDeInfo)
      .catch(() => setDeInfo({ de: "other", exec_path: "portunus" }));
    invoke<boolean>("get_autostart")
      .then(setAutostart)
      .catch(() => setAutostart(false));
  }, []);

  const depOk = (id?: string) => !id || deps == null || deps[id] === true;

  const toggleAutostart = (on: boolean) => {
    setAutostart(on); // optimistic
    setAutostartError(false);
    invoke("set_autostart", { enabled: on }).catch(e => {
      console.error("[onboarding] set_autostart failed:", e);
      setAutostart(!on);
      setAutostartError(true);
    });
  };

  const choosePreset = (p: Preset) => {
    if (p.requires && !depOk(p.requires)) return; // blocked: dependency missing
    setPreset(p.id);
    setDraft(d => ({
      ...d,
      content: {
        ...d.content,
        enabled: p.id !== "skip",
        dirs: p.dirs,
        ocr_images: p.ocr,
        ocr_pdf_fallback: p.ocr,
      },
    }));
  };

  const go = (next: number) => {
    if (next < 0 || next >= STEPS.length) return;
    setDir(next > step ? 1 : -1);
    setStep(next);
  };

  // Commit the wizard: persist the draft and mark onboarding done. Used by both
  // "Finish" and "Skip setup" - skipping early still keeps whatever the user
  // already chose rather than throwing it away.
  const commit = async () => {
    if (finishing) return;
    setFinishing(true);
    setSaveError(false);
    const final: Config = { ...draft, general: { ...draft.general, onboarding_completed: true } };
    try {
      await invoke("save_config", { config: final });
    } catch (e) {
      // Don't mark onboarding done or close - let the user retry. Esc still
      // hides Portunus (onboarding returns next launch) if saving keeps failing.
      console.error("[onboarding] save_config failed:", e);
      setSaveError(true);
      setFinishing(false);
      return;
    }
    // Kick off the first content index only when there's something to index.
    if (final.content.enabled && final.content.dirs.length > 0) {
      invoke("trigger_full_reindex", { full: false }).catch(() => {});
    }
    onComplete();
  };

  const commitRef = useRef(commit);
  commitRef.current = commit;

  const isLast = step === STEPS.length - 1;

  // Keyboard-drive the wizard: Enter advances/commits, Esc dismisses, Tab is
  // trapped inside the modal. Matches the keyboard-first launcher.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Tab") {
        const card = cardRef.current;
        if (!card) return;
        const list = Array.from(
          card.querySelectorAll<HTMLElement>(
            'button:not([disabled]), input:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
          ),
        ).filter(el => el.offsetParent !== null);
        if (list.length === 0) return;
        const first = list[0];
        const last = list[list.length - 1];
        const active = document.activeElement as HTMLElement | null;
        if (active && !card.contains(active)) {
          e.preventDefault();
          first.focus();
        } else if (e.shiftKey && active === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && active === last) {
          e.preventDefault();
          first.focus();
        }
        return;
      }
      if (finishing) return;
      if (e.key === "Enter") {
        const t = e.target as HTMLElement | null;
        if (t?.tagName === "INPUT" || t?.tagName === "TEXTAREA") return; // input owns Enter
        e.preventDefault();
        if (isLast) commitRef.current(); else go(step + 1);
      } else if (e.key === "Escape") {
        // Esc hides Portunus without dismissing onboarding — the wizard stays
        // mounted and reappears at the same step on the next --show.
        e.preventDefault();
        e.stopImmediatePropagation(); // don't let the launcher's Esc also fire
        invoke("hide_window");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [step, isLast, finishing]); // eslint-disable-line react-hooks/exhaustive-deps

  // Focus the primary action on each step so Enter has an obvious target.
  useEffect(() => { primaryRef.current?.focus(); }, [step]);

  return (
    <div className="onb-overlay" role="dialog" aria-modal="true" aria-label="Welcome to Portunus">
      <div className="onb-card" ref={cardRef}>
        <div className="onb-rail">
          {STEPS.map((label, i) => (
            <button
              key={label}
              type="button"
              className={`onb-dot${i === step ? " is-active" : ""}${i < step ? " is-done" : ""}`}
              onClick={() => i < step && go(i)}
              aria-label={`Step ${i + 1}: ${label}`}
              disabled={i > step}
            >
              <span className="onb-dot-fill" />
            </button>
          ))}
        </div>

        <div className="onb-stage">
          <div key={step} className={`onb-step ${dir === 1 ? "onb-step--fwd" : "onb-step--back"}`}>
            {step === 0 && <WelcomeStep version={version} />}
            {step === 1 && (
              <DesktopStep
                info={deInfo}
                autostart={autostart}
                autostartError={autostartError}
                onAutostart={toggleAutostart}
              />
            )}
            {step === 2 && (
              <ContentStep preset={preset} deps={deps} depOk={depOk} onPreset={choosePreset} />
            )}
          </div>
        </div>

        {saveError && (
          <div className="onb-warn" role="alert" style={{ marginTop: 12 }}>
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
              <line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>
            </svg>
            <span>Couldn’t save your settings. Check permissions on <code className="onb-inline">~/.config/portunus</code> and try again, or press <kbd>Esc</kbd> to continue for now.</span>
          </div>
        )}

        <div className="onb-nav">
          <button type="button" className="onb-btn onb-btn--ghost" onClick={() => commit()} disabled={finishing}>
            Skip setup
          </button>
          <div className="onb-nav-right">
            {step > 0 && (
              <button type="button" className="onb-btn onb-btn--quiet" onClick={() => go(step - 1)} disabled={finishing}>
                Back
              </button>
            )}
            {isLast ? (
              <button ref={primaryRef} type="button" className="onb-btn onb-btn--primary" onClick={() => commit()} disabled={finishing}>
                {finishing ? "Finishing…" : "Finish"}
              </button>
            ) : (
              <button ref={primaryRef} type="button" className="onb-btn onb-btn--primary" onClick={() => go(step + 1)}>
                {step === 0 ? "Get started" : "Continue"}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

// ── steps ─────────────────────────────────────────────────────────────────────

function WelcomeStep({ version }: { version: string }) {
  return (
    <div className="onb-welcome onb-welcome--compact">
      <div className="onb-glyph onb-rise" style={{ animationDelay: "40ms" }}>
        <svg viewBox="0 0 1024 1024" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round">
          <path d="M 280 824 L 280 360 A 232 232 0 0 1 744 360 L 744 824" strokeWidth="22" opacity="0.45" />
          <path d="M 332 824 L 332 464 A 180 180 0 0 1 692 464 L 692 824" strokeWidth="22" opacity="0.72" />
          <path d="M 384 824 L 384 568 A 128 128 0 0 1 640 568 L 640 824" strokeWidth="22" />
          <circle cx="512" cy="780" r="20" fill="currentColor" stroke="none" />
        </svg>
        <span className="onb-glyph-glow" aria-hidden />
      </div>
      <h1 className="onb-title onb-rise" style={{ animationDelay: "100ms" }}>
        Welcome to <span className="onb-brand">Portunus</span>
      </h1>
      <p className="onb-lede onb-rise" style={{ animationDelay: "160ms" }}>
        One search box for everything below. Everything is on by default; tune it later in Settings.
      </p>
      <div className="onb-prov-grid onb-prov-grid--tour">
        {FEATURES.map((f, i) => (
          <div key={f.id} className="onb-prov onb-rise" style={{ animationDelay: `${200 + i * 40}ms` }}>
            <span className="onb-prov-icon">{f.icon}</span>
            <div className="onb-prov-body">
              <div className="onb-prov-head">
                <span className="onb-prov-name">{f.name}</span>
              </div>
              <div className="onb-prov-desc">{f.desc}</div>
              {f.example && <code className="onb-prov-ex">{f.example}</code>}
            </div>
          </div>
        ))}
      </div>
      <div className="onb-kbd-row onb-rise" style={{ animationDelay: "520ms" }}>
        Type to search · <kbd>↑</kbd><kbd>↓</kbd> to move · <kbd>↵</kbd> to launch · <kbd>Esc</kbd> to dismiss
        {version && <span className="onb-version-inline">v{version}</span>}
      </div>
    </div>
  );
}

function Snippet({ where, text, caveat }: HotkeyGuide) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    invoke("copy_text", { text }).catch(() =>
      navigator.clipboard.writeText(text).catch(() => {}),
    );
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };
  return (
    <div className="onb-snippet-block">
      <div className="onb-snippet-where">{where}</div>
      <div className="onb-snippet">
        <pre className="onb-snippet-code">{text}</pre>
        <button type="button" className="onb-snippet-copy" onClick={copy}>
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      {caveat && <div className="onb-snippet-caveat">{caveat}</div>}
    </div>
  );
}

function DesktopStep({
  info, autostart, autostartError, onAutostart,
}: {
  info: DeSetupInfo | null;
  autostart: boolean | null;
  autostartError: boolean;
  onAutostart: (on: boolean) => void;
}) {
  if (!info) return null; // probe resolves in ms; a spinner would only flash
  const guide = DE_GUIDES[info.de] ?? DE_GUIDES.other;
  const hotkey = guide.hotkey(info.exec_path);
  const autostartSnippet = guide.autostart?.(info.exec_path);

  return (
    <div className="onb-section">
      <h2 className="onb-h2">Wire it into {guide.label}</h2>
      <p className="onb-sub">
        Portunus runs in the background and appears when something runs{" "}
        <code className="onb-inline">portunus --show</code>. Bind that to a key:
      </p>

      <div className="onb-rise" style={{ animationDelay: "60ms" }}>
        {info.de !== "other" && (
          <span className="onb-chip onb-chip--ok onb-de-chip">detected: {guide.label}</span>
        )}
        <Snippet {...hotkey} />
      </div>

      <div className="onb-rise" style={{ animationDelay: "140ms" }}>
        {guide.xdgAutostart ? (
          <>
            <div className="onb-content-toggle">
              <div>
                <div className="onb-ct-name">Start at login</div>
                <div className="onb-ct-desc">Adds a desktop entry to <code className="onb-inline">~/.config/autostart</code>.</div>
              </div>
              <Toggle
                label="Start at login"
                checked={autostart === true}
                onChange={onAutostart}
                disabled={autostart === null}
              />
            </div>
            {autostartError && (
              <div className="onb-warn onb-warn--muted" role="alert">
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
                </svg>
                <span>Couldn’t update <code className="onb-inline">~/.config/autostart/portunus.desktop</code> - check permissions.</span>
              </div>
            )}
          </>
        ) : (
          autostartSnippet && (
            <>
              <div className="onb-snippet-title">Start at login</div>
              <Snippet {...autostartSnippet} />
            </>
          )
        )}
      </div>

      <p className="onb-footnote onb-rise" style={{ animationDelay: "220ms" }}>
        Autostart can be changed anytime in Settings → General.
      </p>
    </div>
  );
}

function ContentStep({
  preset, deps, depOk, onPreset,
}: {
  preset: PresetId;
  deps: Record<string, boolean> | null;
  depOk: (id?: string) => boolean;
  onPreset: (p: Preset) => void;
}) {
  const showOcrWarning = preset === "ocr";

  return (
    <div className="onb-section">
      <h2 className="onb-h2">Search inside your files</h2>
      <p className="onb-sub">
        Content search builds a full-text index so you can find a phrase buried in a
        document. Press <code className="onb-inline">Tab</code> in the launcher to use it.
      </p>

      <div className="onb-preset-grid">
        {PRESETS.map((p, i) => {
          // Disable presets with a hard dependency until the probe resolves,
          // so a fast click can't apply OCR before we know tesseract is present.
          const blocked = !!p.requires && (deps == null || !depOk(p.requires));
          return (
            <button
              key={p.id}
              type="button"
              className={`onb-preset onb-rise${preset === p.id ? " is-selected" : ""}${blocked ? " is-blocked" : ""}`}
              style={{ animationDelay: `${80 + i * 60}ms` }}
              onClick={() => onPreset(p)}
              disabled={blocked}
            >
              <div className="onb-preset-head">
                <span className="onb-preset-name">{p.name}</span>
                {blocked && <span className="onb-chip onb-chip--warn">needs {p.requires}</span>}
                <span className="onb-preset-check" aria-hidden>
                  <svg width="9" height="9" viewBox="0 0 10 10" fill="none">
                    <polyline points="1.5,5 4,7.5 8.5,2.5" stroke="var(--text-on-accent)" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"/>
                  </svg>
                </span>
              </div>
              <div className="onb-preset-blurb">{p.blurb}</div>
            </button>
          );
        })}
      </div>

      {showOcrWarning && (
        <div className="onb-warn onb-rise" style={{ animationDelay: "60ms" }}>
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
            <line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>
          </svg>
          <span>OCR scans images and scanned PDFs page by page, so the first index may take <strong>several minutes</strong> depending on how many files you have.</span>
        </div>
      )}

      {preset !== "skip" && !depOk("poppler") && (
        <div className="onb-warn onb-warn--muted">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
          </svg>
          <span><strong>poppler</strong> isn’t installed, so PDF text extraction will be skipped until you add it. Other file types still index.</span>
        </div>
      )}

      <p className="onb-footnote onb-rise" style={{ animationDelay: "200ms" }}>
        You can change folders, file types, OCR and languages anytime in Settings → Content.
      </p>
    </div>
  );
}
