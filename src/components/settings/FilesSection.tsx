import { useState, useEffect, useRef, KeyboardEvent } from "react";
import { Config, DirEntry } from "../../types";
import DirRow from "./DirRow";
import Toggle from "./Toggle";

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

  const setFiles = (patch: Partial<Config["files"]>) =>
    onChange({ ...config, files: { ...config.files, ...patch } });
  const setDirs = (dirs: DirEntry[]) => setFiles({ dirs });

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
          <DirRow
            key={i}
            path={dir.path}
            depth={dir.depth}
            onPathChange={path => updateDir(i, { path })}
            onDepthChange={depth => updateDir(i, { depth })}
            onRemove={() => removeDir(i)}
          />
        ))}

        {draft !== null && (
          <DirRow
            draft
            inputRef={draftInputRef}
            path={draft.path}
            depth={draft.depth}
            onPathChange={path => setDraft({ ...draft, path })}
            onDepthChange={depth => setDraft({ ...draft, depth })}
            onRemove={() => setDraft(null)}
            onKeyDown={onDraftKey}
            onCommit={commitDraft}
            onDiscard={() => setDraft(null)}
          />
        )}

        {draft === null && (
          <button className="settings-dir-add" onClick={() => setDraft({ path: "", depth: 2 })}>
            <span style={{ fontSize: 14, lineHeight: 1 }}>+</span> Add directory
          </button>
        )}
      </div>

      <div className="settings-field">
        <div className="settings-field-label">
          <div className="settings-field-name">Show dotfiles</div>
          <div className="settings-field-desc">Include files and directories whose name or path starts with a dot (e.g. .config, .bashrc). Hidden by default.</div>
        </div>
        <div className="settings-field-control">
          <Toggle label="Show dotfiles" checked={config.files.show_dotfiles} onChange={v => setFiles({ show_dotfiles: v })} />
        </div>
      </div>
    </div>
  );
}
