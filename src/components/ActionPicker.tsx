import { useEffect, useMemo, useRef, useState } from "react";
import type { ExtAction, SearchResult } from "../types";

interface Props {
  result: SearchResult;
  onRun: (action: ExtAction) => void;
  onClose: () => void;
}

/**
 * Popover listing an extension result's actions (Alt+Enter). Modal like
 * Quicklook: it owns the keyboard while open (capture-phase listener so the
 * launcher's own handler never sees the keys) and pins the result it was
 * opened for. Typing filters; ↑↓ + Enter or a bare digit runs an action.
 */
export default function ActionPicker({ result, onRun, onClose }: Props) {
  const [filter, setFilter] = useState("");
  const [index, setIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const selectedRef = useRef<HTMLDivElement>(null);

  const actions = useMemo(() => {
    const all = result.ext?.actions ?? [];
    const f = filter.trim().toLowerCase();
    if (!f) return all;
    return all.filter(a => a.label.toLowerCase().includes(f) || a.hint?.toLowerCase().includes(f));
  }, [result, filter]);

  useEffect(() => setIndex(0), [filter]);
  useEffect(() => inputRef.current?.focus(), []);
  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" });
  }, [index]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Capture phase + stopImmediatePropagation: the launcher's window-level
      // handler (navigation, launch, Esc-hides-window) must stay inert.
      e.stopImmediatePropagation();
      if (e.key === "Escape" || (e.altKey && e.key === "Enter")) {
        e.preventDefault();
        onClose();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setIndex(i => (actions.length ? (i + 1) % actions.length : 0));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setIndex(i => (actions.length ? (i - 1 + actions.length) % actions.length : 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const action = actions[index];
        if (action) onRun(action);
      } else if (!filter && e.key >= "1" && e.key <= "9" && !e.ctrlKey && !e.altKey && !e.metaKey) {
        // Bare digits are shortcuts only while the filter is empty; once the
        // user starts filtering, digits type into the filter like any char.
        const action = actions[parseInt(e.key) - 1];
        if (action) {
          e.preventDefault();
          onRun(action);
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [actions, index, filter, onRun, onClose]);

  return (
    <div className="action-picker">
      <div className="action-picker-header">
        <span className="action-picker-title">{result.title}</span>
        <input
          ref={inputRef}
          className="action-picker-filter"
          type="text"
          placeholder="Filter actions…"
          value={filter}
          onChange={e => setFilter(e.target.value)}
          spellCheck={false}
        />
      </div>
      <div className="action-picker-list">
        {actions.length === 0 && <div className="action-picker-empty">No matching actions</div>}
        {actions.map((a, i) => (
          <div
            key={a.id}
            ref={i === index ? selectedRef : undefined}
            className={`action-picker-row${i === index ? " selected" : ""}`}
            onMouseEnter={() => setIndex(i)}
            onClick={() => onRun(a)}
          >
            <span className="action-picker-label">{a.label}</span>
            {a.hint && <span className="action-picker-hint">{a.hint}</span>}
            {!filter && i < 9 && <kbd className="action-picker-key">{i + 1}</kbd>}
          </div>
        ))}
      </div>
    </div>
  );
}
