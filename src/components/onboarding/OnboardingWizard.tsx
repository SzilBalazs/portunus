import { useEffect, useRef, useState, type ReactElement } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { Config, ContentDirEntry, DepStatus } from "../../types";
import { applyTheme } from "../../theme";
import ThemeGrid from "../settings/ThemeGrid";
import Toggle from "../settings/Toggle";
import "./onboarding.css";

interface Props {
  config: Config;
  onComplete: () => void;
}

const STEPS = ["Welcome", "Theme", "Providers", "Content"] as const;

// ── provider tour data ──────────────────────────────────────────────────────

type ProviderKey = "apps" | "files" | "calc";

interface ProviderCard {
  id: string;
  name: string;
  desc: string;
  example?: string;
  icon: ReactElement;
  /** Config flag this card toggles, if any. */
  toggle?: ProviderKey;
  /** Optional external binary this provider needs to function. */
  dep?: string;
  /** Shown instead of a toggle (e.g. "set up next"). */
  note?: string;
}

const I = (paths: ReactElement) => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">{paths}</svg>
);

const PROVIDERS: ProviderCard[] = [
  { id: "apps", name: "Applications", desc: "Launch any installed app", example: "firefox", toggle: "apps",
    icon: I(<><rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></>) },
  { id: "files", name: "Files", desc: "Find files by name, fuzzy-matched", example: "report.pdf", toggle: "files",
    icon: I(<><path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><path d="M13 2v7h7"/></>) },
  { id: "calc", name: "Calculator", desc: "Inline math", example: "1+2*3", toggle: "calc",
    icon: I(<><rect x="4" y="2" width="16" height="20" rx="2"/><line x1="8" y1="6" x2="16" y2="6"/><line x1="8" y1="11" x2="8" y2="11"/><line x1="12" y1="11" x2="12" y2="11"/><line x1="16" y1="11" x2="16" y2="11"/><line x1="8" y1="15" x2="8" y2="15"/><line x1="12" y1="15" x2="12" y2="15"/><line x1="16" y1="15" x2="16" y2="18"/></>) },
  { id: "dict", name: "Dictionary", desc: "Word definitions", example: "define lucid", dep: "dict",
    icon: I(<><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></>) },
  { id: "clipboard", name: "Clipboard", desc: "Search your clipboard history", example: "clipboard api", dep: "cliphist",
    icon: I(<><rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/></>) },
  { id: "timer", name: "Timers", desc: "Set quick timers with a chime", example: "timer 5m tea",
    icon: I(<><circle cx="12" cy="13" r="8"/><path d="M12 9v4l2 2"/><path d="M9 2h6"/></>) },
  { id: "content", name: "Content search", desc: "Full-text search inside files", example: "! invoice", note: "Set up next →",
    icon: I(<><circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/><path d="M8 11h6"/><path d="M11 8v6"/></>) },
];

// ── content presets ──────────────────────────────────────────────────────────

type PresetId = "minimal" | "docs" | "ocr";

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
  { id: "minimal", name: "Just enable it", blurb: "Turn it on with no folders yet; add directories later in Settings.", dirs: [], ocr: false },
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
  const [preset, setPreset] = useState<PresetId | null>(null);
  const [screenshotDir, setScreenshotDir] = useState<string | null>(null);
  const [finishing, setFinishing] = useState(false);
  // True once the user intentionally finishes, so unmount cleanup keeps their theme.
  const committedRef = useRef(false);
  const primaryRef = useRef<HTMLButtonElement>(null);

  useEffect(() => { getVersion().then(setVersion); }, []);

  // Probe optional system dependencies once so provider/content steps can warn.
  useEffect(() => {
    invoke<DepStatus[]>("check_dependencies")
      .then(list => setDeps(Object.fromEntries(list.map(d => [d.id, d.available]))))
      .catch(() => setDeps({}));
  }, []);

  // Live-apply the theme as the user browses, so the whole wizard reskins instantly.
  useEffect(() => { applyTheme(draft.appearance); }, [draft.appearance]);

  // Restore the user's real saved theme if they bail out without finishing.
  // On an intentional finish (committedRef) the chosen theme is kept instead.
  useEffect(() => () => { if (!committedRef.current) applyTheme(config.appearance); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const depOk = (id?: string) => !id || deps == null || deps[id] === true;

  const setProviders = (patch: Partial<Config["providers"]>) =>
    setDraft(d => ({ ...d, providers: { ...d.providers, ...patch } }));

  const choosePreset = (p: Preset) => {
    if (p.requires && !depOk(p.requires)) return; // blocked: dependency missing
    if (preset === p.id) return; // already active - re-selecting would wipe an added screenshot folder
    setPreset(p.id);
    setScreenshotDir(null); // preset rewrites dirs, so drop any added screenshot folder
    setDraft(d => ({
      ...d,
      content: { ...d.content, enabled: true, dirs: p.dirs, ocr_images: p.ocr, ocr_pdf_fallback: p.ocr },
    }));
  };

  // OCR makes any text in a screenshot searchable; let the user point us at their folder.
  const addScreenshotDir = (raw: string) => {
    const path = raw.trim();
    if (!path) return;
    const entry: ContentDirEntry = { path, depth: 2, extensions: ["png", "jpg", "jpeg"] };
    setDraft(d => ({
      ...d,
      content: { ...d.content, dirs: [...d.content.dirs.filter(x => x.path !== path), entry] },
    }));
    setScreenshotDir(path);
  };

  const removeScreenshotDir = () => {
    setDraft(d => ({
      ...d,
      content: { ...d.content, dirs: d.content.dirs.filter(x => x.path !== screenshotDir) },
    }));
    setScreenshotDir(null);
  };

  const toggleContent = (on: boolean) => {
    if (on) {
      // Default to the recommended preset when first enabling.
      choosePreset(PRESETS[1]);
    } else {
      setPreset(null);
      setDraft(d => ({ ...d, content: { ...d.content, enabled: false } }));
    }
  };

  const go = (next: number) => {
    if (next < 0 || next >= STEPS.length) return;
    setDir(next > step ? 1 : -1);
    setStep(next);
  };

  const finish = async (skipped: boolean) => {
    if (finishing) return;
    setFinishing(true);
    // Skip = leave everything at its saved defaults; only mark onboarding done.
    // Finish = persist the draft (theme, providers, content) and keep it applied.
    if (!skipped) committedRef.current = true;
    const base = skipped ? config : draft;
    const final: Config = { ...base, general: { ...base.general, onboarding_completed: true } };
    try {
      await invoke("save_config", { config: final });
      // Kick off the first content index only when there's something to index.
      if (!skipped && final.content.enabled && final.content.dirs.length > 0) {
        invoke("trigger_full_reindex").catch(() => {});
      }
    } catch {
      /* even if saving fails, don't trap the user behind the wizard */
    }
    onComplete();
  };
  const finishRef = useRef(finish);
  finishRef.current = finish;

  const isLast = step === STEPS.length - 1;

  // Keyboard-drive the wizard: Enter advances/finishes, Esc skips. Matches the
  // keyboard-first launcher and the keys advertised on the Welcome screen.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (finishing) return;
      if (e.key === "Enter") {
        const t = e.target as HTMLElement | null;
        if (t?.tagName === "INPUT" || t?.tagName === "TEXTAREA") return; // input owns Enter
        e.preventDefault();
        if (isLast) finishRef.current(false); else go(step + 1);
      } else if (e.key === "Escape") {
        e.preventDefault();
        finishRef.current(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [step, isLast, finishing]); // eslint-disable-line react-hooks/exhaustive-deps

  // Focus the primary action on each step so Enter has an obvious target.
  useEffect(() => { primaryRef.current?.focus(); }, [step]);

  return (
    <div className="onb-overlay" role="dialog" aria-modal="true" aria-label="Welcome to Portunus">
      <div className="onb-card">
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
              <ThemeStep
                value={draft.appearance.theme}
                onSelect={id => setDraft(d => ({ ...d, appearance: { ...d.appearance, theme: id } }))}
              />
            )}
            {step === 2 && (
              <ProvidersStep providers={draft.providers} deps={deps} depOk={depOk} onToggle={setProviders} />
            )}
            {step === 3 && (
              <ContentStep
                enabled={draft.content.enabled}
                preset={preset}
                deps={deps}
                depOk={depOk}
                onToggle={toggleContent}
                onPreset={choosePreset}
                screenshotDir={screenshotDir}
                onAddScreenshots={addScreenshotDir}
                onRemoveScreenshots={removeScreenshotDir}
              />
            )}
          </div>
        </div>

        <div className="onb-nav">
          <button type="button" className="onb-btn onb-btn--ghost" onClick={() => finish(true)} disabled={finishing}>
            Skip setup
          </button>
          <div className="onb-nav-right">
            {step > 0 && (
              <button type="button" className="onb-btn onb-btn--quiet" onClick={() => go(step - 1)} disabled={finishing}>
                Back
              </button>
            )}
            {isLast ? (
              <button ref={primaryRef} type="button" className="onb-btn onb-btn--primary" onClick={() => finish(false)} disabled={finishing}>
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
    <div className="onb-welcome">
      <div className="onb-glyph onb-rise" style={{ animationDelay: "40ms" }}>
        <svg viewBox="0 0 1024 1024" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round">
          <path d="M 280 824 L 280 360 A 232 232 0 0 1 744 360 L 744 824" strokeWidth="22" opacity="0.45" />
          <path d="M 332 824 L 332 464 A 180 180 0 0 1 692 464 L 692 824" strokeWidth="22" opacity="0.72" />
          <path d="M 384 824 L 384 568 A 128 128 0 0 1 640 568 L 640 824" strokeWidth="22" />
          <circle cx="512" cy="780" r="20" fill="currentColor" stroke="none" />
        </svg>
        <span className="onb-glyph-glow" aria-hidden />
      </div>
      <h1 className="onb-title onb-rise" style={{ animationDelay: "120ms" }}>
        Welcome to <span className="onb-brand">Portunus</span>
      </h1>
      <p className="onb-lede onb-rise" style={{ animationDelay: "200ms" }}>
        A fast, keyboard-first launcher for apps, files, math, definitions and the
        contents of your documents, all from one search box.
      </p>
      <div className="onb-kbd-row onb-rise" style={{ animationDelay: "280ms" }}>
        Type to search · <kbd>↑</kbd><kbd>↓</kbd> to move · <kbd>↵</kbd> to launch · <kbd>Esc</kbd> to dismiss
      </div>
      {version && <div className="onb-version onb-rise" style={{ animationDelay: "340ms" }}>v{version}</div>}
    </div>
  );
}

function ThemeStep({ value, onSelect }: { value: string; onSelect: (id: string) => void }) {
  return (
    <div className="onb-section">
      <h2 className="onb-h2">Pick a theme</h2>
      <p className="onb-sub">Click any theme to preview it live. You can fine-tune this later in Settings → Appearance.</p>
      <div className="onb-rise" style={{ animationDelay: "80ms" }}>
        <ThemeGrid value={value} onSelect={onSelect} />
      </div>
    </div>
  );
}

function ProvidersStep({
  providers, deps, depOk, onToggle,
}: {
  providers: Config["providers"];
  deps: Record<string, boolean> | null;
  depOk: (id?: string) => boolean;
  onToggle: (patch: Partial<Config["providers"]>) => void;
}) {
  return (
    <div className="onb-section">
      <h2 className="onb-h2">What Portunus can search</h2>
      <p className="onb-sub">Toggle providers on or off. Some need an extra tool installed, shown with a tag.</p>
      <div className="onb-prov-grid">
        {PROVIDERS.map((p, i) => {
          const available = depOk(p.dep);
          const checked = p.toggle ? providers[p.toggle] : true;
          return (
            <div
              key={p.id}
              className={`onb-prov onb-rise${!available ? " is-unavailable" : ""}`}
              style={{ animationDelay: `${60 + i * 45}ms` }}
            >
              <span className="onb-prov-icon">{p.icon}</span>
              <div className="onb-prov-body">
                <div className="onb-prov-head">
                  <span className="onb-prov-name">{p.name}</span>
                  {p.dep && !available && deps != null && (
                    <span className="onb-chip onb-chip--warn">needs {p.dep}</span>
                  )}
                </div>
                <div className="onb-prov-desc">{p.desc}</div>
                {p.example && <code className="onb-prov-ex">{p.example}</code>}
              </div>
              <div className="onb-prov-ctrl">
                {p.toggle ? (
                  <Toggle label={p.name} checked={checked} onChange={v => onToggle({ [p.toggle!]: v })} />
                ) : p.note ? (
                  <span className="onb-prov-note">{p.note}</span>
                ) : p.dep && !available ? (
                  /* head already shows the "needs {dep}" chip - don't double up */
                  null
                ) : (
                  <span className={`onb-chip${available ? " onb-chip--ok" : " onb-chip--warn"}`}>
                    {available ? "ready" : "optional"}
                  </span>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function ContentStep({
  enabled, preset, deps, depOk, onToggle, onPreset,
  screenshotDir, onAddScreenshots, onRemoveScreenshots,
}: {
  enabled: boolean;
  preset: PresetId | null;
  deps: Record<string, boolean> | null;
  depOk: (id?: string) => boolean;
  onToggle: (on: boolean) => void;
  onPreset: (p: Preset) => void;
  screenshotDir: string | null;
  onAddScreenshots: (path: string) => void;
  onRemoveScreenshots: () => void;
}) {
  const popplerMissing = !depOk("poppler");
  const showOcrWarning = preset === "ocr";
  const [screenshotInput, setScreenshotInput] = useState("~/Pictures/Screenshots");

  return (
    <div className="onb-section">
      <h2 className="onb-h2">Search inside your files</h2>
      <p className="onb-sub">
        Content search builds a full-text index so you can find a phrase buried in a
        document. Prefix a query with <code className="onb-inline">!</code> to use it.
      </p>

      <div className="onb-content-toggle onb-rise" style={{ animationDelay: "60ms" }}>
        <div>
          <div className="onb-ct-name">Enable content search</div>
          <div className="onb-ct-desc">The index builds in the background, so you can keep using Portunus.</div>
        </div>
        <Toggle label="Enable content search" checked={enabled} onChange={onToggle} />
      </div>

      {enabled && (
        <>
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
                  style={{ animationDelay: `${120 + i * 60}ms` }}
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

          {showOcrWarning && (
            <div className="onb-hint onb-rise" style={{ animationDelay: "120ms" }}>
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M12 3v3m0 12v3M5.6 5.6l2.1 2.1m8.6 8.6 2.1 2.1M3 12h3m12 0h3M5.6 18.4l2.1-2.1m8.6-8.6 2.1-2.1"/>
              </svg>
              <div className="onb-hint-body">
                <div className="onb-hint-head"><strong>Neat:</strong> index your screenshots folder. OCR makes any text in a screenshot searchable.</div>
                {screenshotDir ? (
                  <div className="onb-hint-added">
                    <span className="onb-hint-path">{screenshotDir}</span>
                    <button type="button" className="onb-hint-remove" onClick={onRemoveScreenshots}>Remove</button>
                  </div>
                ) : (
                  <div className="onb-hint-row">
                    <div className="onb-hint-field">
                      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <path d="M12 20h9"/><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z"/>
                      </svg>
                      <input
                        className="onb-hint-input"
                        type="text"
                        value={screenshotInput}
                        placeholder="~/Pictures/Screenshots"
                        onChange={e => setScreenshotInput(e.target.value)}
                        onKeyDown={e => { if (e.key === "Enter") onAddScreenshots(screenshotInput); }}
                        aria-label="Screenshots folder path"
                      />
                    </div>
                    <button
                      type="button"
                      className="onb-hint-add"
                      onClick={() => onAddScreenshots(screenshotInput)}
                      disabled={!screenshotInput.trim()}
                    >
                      Add
                    </button>
                  </div>
                )}
              </div>
            </div>
          )}

          {popplerMissing && deps_note(depOk)}

          <p className="onb-footnote onb-rise" style={{ animationDelay: "240ms" }}>
            You can change folders, file types, OCR and languages anytime in Settings → Content.
          </p>
        </>
      )}
    </div>
  );
}

// Small helper kept outside JSX for readability: poppler-missing note.
function deps_note(depOk: (id?: string) => boolean) {
  if (depOk("poppler")) return null;
  return (
    <div className="onb-warn onb-warn--muted">
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
      </svg>
      <span><strong>poppler</strong> isn’t installed, so PDF text extraction will be skipped until you add it. Other file types still index.</span>
    </div>
  );
}
