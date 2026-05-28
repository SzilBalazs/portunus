import { useState, useEffect, useRef, KeyboardEvent } from "react";
import { Config, DirEntry } from "../../types";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

export default function FilesSection({ config, onChange }: Props) {
  const [draft, setDraft] = useState<DirEntry | null>(null);
  const draftInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (draft !== null) draftInputRef.current?.focus();
  }, [draft !== null]);

  const setDirs = (dirs: DirEntry[]) =>
    onChange({ ...config, files: { ...config.files, dirs } });

  const updateDir = (i: number, patch: Partial<DirEntry>) => {
    const next = config.files.dirs.map((d, idx) => idx === i ? { ...d, ...patch } : d);
    setDirs(next);
  };

  const removeDir = (i: number) =>
    setDirs(config.files.dirs.filter((_, idx) => idx !== i));

  const commitDraft = () => {
    if (!draft || draft.path.trim() === "") return;
    setDirs([...config.files.dirs, draft]);
    setDraft(null);
  };

  const onDraftKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") { e.preventDefault(); commitDraft(); }
    if (e.key === "Escape") { e.preventDefault(); setDraft(null); }
  };

  return (
    <div>
      <div className="settings-section-header">
        <div className="settings-section-name">Files</div>
        <div className="settings-section-desc">Directories indexed by the file provider.</div>
      </div>

      <div className="settings-section-note">
        Each directory is crawled up to the specified depth. Use <code>~/</code> for paths relative to your home directory.
      </div>

      <div className="settings-dir-list">
        {config.files.dirs.map((dir, i) => (
          <div className="settings-dir-row" key={i}>
            <input
              className="settings-dir-path"
              value={dir.path}
              placeholder="~/path/to/dir"
              onChange={e => updateDir(i, { path: e.target.value })}
            />
            <div className="settings-dir-depth">
              <button className="settings-dir-depth-btn" onClick={() => updateDir(i, { depth: Math.max(1, dir.depth - 1) })}>−</button>
              <span className="settings-dir-depth-val" title="Search depth">{dir.depth}</span>
              <button className="settings-dir-depth-btn" onClick={() => updateDir(i, { depth: Math.min(10, dir.depth + 1) })}>+</button>
            </div>
            <button className="settings-dir-remove" onClick={() => removeDir(i)} title="Remove">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14H6L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4h6v2"/>
              </svg>
            </button>
          </div>
        ))}

        {draft !== null && (
          <div className="settings-dir-row settings-dir-row--draft">
            <input
              ref={draftInputRef}
              className="settings-dir-path"
              value={draft.path}
              placeholder="~/path/to/dir"
              onChange={e => setDraft({ ...draft, path: e.target.value })}
              onKeyDown={onDraftKey}
            />
            <div className="settings-dir-depth">
              <button className="settings-dir-depth-btn" onClick={() => setDraft({ ...draft, depth: Math.max(1, draft.depth - 1) })}>−</button>
              <span className="settings-dir-depth-val" title="Search depth">{draft.depth}</span>
              <button className="settings-dir-depth-btn" onClick={() => setDraft({ ...draft, depth: Math.min(10, draft.depth + 1) })}>+</button>
            </div>
            <button
              className="settings-dir-confirm"
              onClick={commitDraft}
              disabled={draft.path.trim() === ""}
              title="Confirm"
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="20 6 9 17 4 12"/>
              </svg>
            </button>
            <button className="settings-dir-remove" onClick={() => setDraft(null)} title="Discard">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
              </svg>
            </button>
          </div>
        )}

        {draft === null && (
          <button className="settings-dir-add" onClick={() => setDraft({ path: "", depth: 2 })}>
            <span style={{ fontSize: 14, lineHeight: 1 }}>+</span> Add directory
          </button>
        )}
      </div>
    </div>
  );
}
