import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Config, ContentDirEntry, DirEstimate } from "../../types";
import Toggle from "./Toggle";
import DirRow from "./DirRow";
import SectionHeader from "./SectionHeader";
import SettingsGroup from "./SettingsGroup";
import SettingsField from "./SettingsField";
import NumberStepper from "./NumberStepper";
import ExtensionEditor from "./ExtensionEditor";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
  pendingReindex: boolean;
  reindexProgress: { indexed: number; total: number } | null;
  reindexError: string | null;
  onApply: () => void;
  onRevert: () => void;
}

const nf = new Intl.NumberFormat();

/** Human-readable single duration, e.g. "<1 s", "12 s", "4 min", "1.5 h". */
function fmtSecs(secs: number): string {
  if (secs < 1) return "<1 s";
  if (secs < 60) return `${Math.round(secs)} s`;
  if (secs < 3600) return `${Math.round(secs / 60)} min`;
  return `${(secs / 3600).toFixed(1)} h`;
}

/** Collapses a min/max range to one value when both render the same. */
function fmtRange(min: number, max: number): string {
  const a = fmtSecs(min), b = fmtSecs(max);
  return a === b ? `~${a}` : `~${a}-${b}`;
}

/**
 * Live, debounced pre-index estimate for one directory. Re-fetches whenever the
 * path, depth, effective extensions, or the indexing flags change. A monotonic
 * request id guards against a slow earlier walk overwriting a newer result.
 */
function useDirEstimate(
  path: string,
  depth: number,
  extensions: string[],
  maxFileBytes: number,
  ocrImages: boolean,
  ocrPdfFallback: boolean,
  threads: number,
): { estimate: DirEstimate | null; loading: boolean } {
  const [estimate, setEstimate] = useState<DirEstimate | null>(null);
  const [loading, setLoading] = useState(false);
  const reqId = useRef(0);
  const extKey = JSON.stringify(extensions);

  useEffect(() => {
    if (path.trim() === "") {
      setEstimate(null);
      setLoading(false);
      return;
    }
    const id = ++reqId.current;
    setLoading(true);
    const timer = setTimeout(async () => {
      try {
        const result = await invoke<DirEstimate>("estimate_dir_index", {
          path, depth, extensions, maxFileBytes, ocrImages, ocrPdfFallback, threads,
        });
        if (id === reqId.current) { setEstimate(result); setLoading(false); }
      } catch {
        if (id === reqId.current) { setEstimate(null); setLoading(false); }
      }
    }, 400);
    return () => clearTimeout(timer);
    // extKey stands in for the extensions array identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, depth, extKey, maxFileBytes, ocrImages, ocrPdfFallback, threads]);

  return { estimate, loading };
}

function DirEstimateRow({ estimate, loading }: { estimate: DirEstimate | null; loading: boolean }) {
  if (loading && !estimate) {
    return <div className="settings-dir-estimate settings-dir-estimate--loading">estimating…</div>;
  }
  if (!estimate) return null;
  if (estimate.total_files === 0) {
    return <div className="settings-dir-estimate settings-dir-estimate--empty">No matching files found</div>;
  }
  const parts = [`${nf.format(estimate.total_files)} files`];
  if (estimate.pdf_files > 0) parts.push(`${nf.format(estimate.pdf_files)} PDFs`);
  if (estimate.image_files > 0) parts.push(`${nf.format(estimate.image_files)} images`);
  return (
    <div className={`settings-dir-estimate${loading ? " settings-dir-estimate--stale" : ""}`}>
      <span className="settings-dir-estimate-counts">{parts.join(" · ")}</span>
      <span className="settings-dir-estimate-sep">·</span>
      <span className="settings-dir-estimate-time" title="Full-index estimate. PDFs without a text layer are OCR'd, which is far slower, hence the range.">
        {fmtRange(estimate.est_secs_min, estimate.est_secs_max)}
      </span>
    </div>
  );
}

function ContentDirCard({
  dir, globalExts, flags, onChange, onRemove,
}: {
  dir: ContentDirEntry;
  globalExts: string[];
  flags: { maxFileBytes: number; ocrImages: boolean; ocrPdfFallback: boolean; threads: number };
  onChange: (patch: Partial<ContentDirEntry>) => void;
  onRemove: () => void;
}) {
  const effectiveExts = dir.extensions ?? globalExts;
  const { estimate, loading } = useDirEstimate(
    dir.path, dir.depth, effectiveExts,
    flags.maxFileBytes, flags.ocrImages, flags.ocrPdfFallback, flags.threads,
  );

  return (
    <div className="settings-dir-card">
      <DirRow
        path={dir.path}
        depth={dir.depth}
        onPathChange={path => onChange({ path })}
        onDepthChange={depth => onChange({ depth })}
        onRemove={onRemove}
      />
      <div className="settings-dir-card-exts">
        <span className="settings-dir-card-exts-label">Override extensions:</span>
        <ExtensionEditor
          extensions={dir.extensions ?? []}
          onChange={exts => onChange({ extensions: exts.length > 0 ? exts : null })}
        />
      </div>
      <DirEstimateRow estimate={estimate} loading={loading} />
    </div>
  );
}

export default function ContentSection({ config, onChange, pendingReindex, reindexProgress, reindexError, onApply, onRevert }: Props) {
  const cc = config.content;
  const reindexing = reindexProgress != null && reindexProgress.total > 0 && reindexProgress.indexed < reindexProgress.total;
  const pct = reindexProgress && reindexProgress.total > 0
    ? Math.min(100, (reindexProgress.indexed / reindexProgress.total) * 100)
    : 0;

  const set = (patch: Partial<Config["content"]>) =>
    onChange({ ...config, content: { ...config.content, ...patch } });

  const updateDir = (i: number, patch: Partial<ContentDirEntry>) =>
    set({ dirs: cc.dirs.map((d, idx) => idx === i ? { ...d, ...patch } : d) });

  const removeDir = (i: number) => set({ dirs: cc.dirs.filter((_, idx) => idx !== i) });

  // No draft/confirm step: Apply & Reindex is the single commit point, so adding
  // a directory just appends a blank, editable row. Blank rows are filtered out
  // before anything is persisted (see Settings.tsx).
  const addDir = () => set({ dirs: [...cc.dirs, { path: "", depth: 3, extensions: null }] });

  const bytesToMb = (b: number) => +(b / (1024 * 1024)).toFixed(1);
  const mbToBytes = (mb: number) => Math.round(mb * 1024 * 1024);

  const flags = {
    maxFileBytes: cc.max_file_bytes,
    ocrImages: cc.ocr_images,
    ocrPdfFallback: cc.ocr_pdf_fallback,
    threads: cc.threads,
  };

  return (
    <div className="settings-section">
      <SectionHeader
        title="Content"
        desc={<>Full-text search inside files. Press <kbd className="settings-kbd">Tab</kbd> in the launcher to activate.</>}
        master={{ checked: cc.enabled, onChange: v => set({ enabled: v }), label: "Enable content search" }}
      />

      <div className="settings-section-note">
        The AppImage bundles everything content search needs, so PDF extraction and OCR work out of the box. A source build instead uses the system <strong>poppler</strong> (pdftotext/pdftoppm) and <strong>tesseract</strong>. Re-index on demand: <code>portunus --reindex</code>
      </div>

      {reindexError && !reindexing && (
        <div className="settings-reindex-strip settings-reindex-strip--error">
          <svg className="settings-reindex-icon" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
          </svg>
          <span className="settings-reindex-msg">Reindex failed: {reindexError}</span>
          <button className="settings-reindex-apply" onClick={onApply}>Retry</button>
        </div>
      )}

      {pendingReindex && !reindexing && !reindexError && (
        <div className="settings-reindex-strip">
          <svg className="settings-reindex-icon" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
            <line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>
          </svg>
          <span className="settings-reindex-msg">Content changes are staged. Apply them to update the search index.</span>
          <button className="settings-reindex-revert" onClick={onRevert}>Revert</button>
          <button className="settings-reindex-apply" onClick={onApply}>Apply &amp; Reindex</button>
        </div>
      )}

      {reindexing && (
        <div className="settings-reindex-strip settings-reindex-strip--progress">
          <span className="settings-reindex-msg">Reindexing… {reindexProgress!.indexed} / {reindexProgress!.total}</span>
          <div className="settings-reindex-bar">
            <div className="settings-reindex-bar-fill" style={{ width: `${pct}%` }} />
          </div>
        </div>
      )}

      <SettingsGroup title="Indexing">
        <SettingsField name="Max file size" desc="Skip files larger than this limit to avoid long indexing times.">
          <NumberStepper
            label="Max file size"
            value={bytesToMb(cc.max_file_bytes)}
            min={0.5} max={512} step={1}
            suffix="MB" width={64}
            onChange={mb => set({ max_file_bytes: mbToBytes(mb) })}
          />
        </SettingsField>

        <SettingsField name="OCR images" desc="Run OCR on image files such as jpg, png, and webp using Tesseract.">
          <Toggle label="OCR images" checked={cc.ocr_images} onChange={v => set({ ocr_images: v })} />
        </SettingsField>

        <SettingsField name="OCR PDF fallback" desc="Run OCR on PDFs that contain no extractable text layer (scanned documents).">
          <Toggle label="OCR PDF fallback" checked={cc.ocr_pdf_fallback} onChange={v => set({ ocr_pdf_fallback: v })} />
        </SettingsField>

        <SettingsField
          name="OCR language"
          desc={<>Tesseract language code, shared with clipboard image OCR. English (<code>eng</code>) is bundled; other languages need the matching tesseract-data-&lt;lang&gt; data. Combine with <code>+</code> (e.g. <code>eng+hun</code>).</>}
        >
          <input
            className="settings-text-input"
            value={cc.ocr_language}
            onChange={e => set({ ocr_language: e.target.value })}
            placeholder="eng"
            style={{ width: 90 }}
          />
        </SettingsField>

        <SettingsField name="Indexer threads" desc="How many CPU threads index in parallel. 0 = use all cores.">
          <NumberStepper label="Indexer threads" value={cc.threads} min={0} max={64} step={1} onChange={v => set({ threads: Math.round(v) })} />
        </SettingsField>
      </SettingsGroup>

      <SettingsGroup
        title="Indexed file types"
        desc="Press Enter or comma to add an extension. Backspace removes the last one. This is the default for every directory below."
      >
        <ExtensionEditor extensions={cc.extensions} onChange={extensions => set({ extensions })} />
      </SettingsGroup>

      <SettingsGroup
        title="Indexed directories"
        desc="Depth controls recursion. Per-directory extensions override the global list when set. Each card shows a live estimate of how long indexing it would take — narrow the extensions before applying to keep big folders fast."
      >
        <div className="settings-dir-list">
          {cc.dirs.map((dir, i) => (
            <ContentDirCard
              key={i}
              dir={dir}
              globalExts={cc.extensions}
              flags={flags}
              onChange={patch => updateDir(i, patch)}
              onRemove={() => removeDir(i)}
            />
          ))}
          <button className="settings-dir-add" onClick={addDir}>
            <span style={{ fontSize: 14, lineHeight: 1 }}>+</span> Add directory
          </button>
        </div>
      </SettingsGroup>
    </div>
  );
}
