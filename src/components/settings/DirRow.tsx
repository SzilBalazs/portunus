import { KeyboardEvent, RefObject } from "react";

const TrashIcon = () => (
  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14H6L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4h6v2"/>
  </svg>
);

const CheckIcon = () => (
  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
    <polyline points="20 6 9 17 4 12"/>
  </svg>
);

const XIcon = () => (
  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
    <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
  </svg>
);

interface BaseProps {
  path: string;
  depth: number;
  onPathChange: (path: string) => void;
  onDepthChange: (depth: number) => void;
  onRemove: () => void;
}

interface ExistingRowProps extends BaseProps {
  draft?: false;
}

interface DraftRowProps extends BaseProps {
  draft: true;
  inputRef: RefObject<HTMLInputElement | null>;
  onKeyDown: (e: KeyboardEvent<HTMLInputElement>) => void;
  onCommit: () => void;
  onDiscard: () => void;
}

type Props = ExistingRowProps | DraftRowProps;

export default function DirRow(props: Props) {
  const { path, depth, onPathChange, onDepthChange, onRemove } = props;
  const isDraft = props.draft === true;

  return (
    <div className={`settings-dir-row${isDraft ? " settings-dir-row--draft" : ""}`}>
      <input
        ref={isDraft ? (props as DraftRowProps).inputRef : undefined}
        className="settings-dir-path"
        value={path}
        placeholder="~/path/to/dir"
        onChange={e => onPathChange(e.target.value)}
        onKeyDown={isDraft ? (props as DraftRowProps).onKeyDown : undefined}
      />
      <div className="settings-dir-depth">
        <button className="settings-dir-depth-btn" onClick={() => onDepthChange(Math.max(1, depth - 1))}>−</button>
        <span className="settings-dir-depth-val" title="Search depth">{depth}</span>
        <button className="settings-dir-depth-btn" onClick={() => onDepthChange(Math.min(10, depth + 1))}>+</button>
      </div>
      {isDraft ? (
        <>
          <button
            className="settings-dir-confirm"
            onClick={(props as DraftRowProps).onCommit}
            disabled={path.trim() === ""}
            title="Confirm"
          >
            <CheckIcon />
          </button>
          <button className="settings-dir-remove" onClick={(props as DraftRowProps).onDiscard} title="Discard">
            <XIcon />
          </button>
        </>
      ) : (
        <button className="settings-dir-remove" onClick={onRemove} title="Remove">
          <TrashIcon />
        </button>
      )}
    </div>
  );
}
