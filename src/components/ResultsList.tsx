import { Fragment, useEffect, useRef, type CSSProperties } from "react";
import { SearchResult } from "../types";
import { groupLabel, formatBytes, shortenPath } from "../utils";
import ResultIcon from "./ResultIcon";

interface Props {
  results: SearchResult[];
  selectedIndex: number;
  /** Whether the user has entered a meaningful search term (drives the empty state). */
  active: boolean;
  searching?: boolean;
  onSelect: (index: number) => void;
  onLaunch: (result?: SearchResult) => void;
  launchableResults: SearchResult[];
}

export default function ResultsList({ results, selectedIndex, active, searching, onSelect, onLaunch, launchableResults }: Props) {
  const selectedRef = useRef<HTMLDivElement>(null);
  const selectedLabelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // Scroll the row first so it's always visible, then the group label (if the
    // selected row is first-in-group) so its header isn't clipped above the fold.
    // The label is adjacent above the row, so scrolling it back in keeps the row
    // visible too.
    selectedRef.current?.scrollIntoView({ block: "nearest" });
    selectedLabelRef.current?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  return (
    <div className="results-col" role="listbox">
      {active && results.length === 0 && !searching && (
        <div className="results-empty">No results</div>
      )}
      {results.map((result, i) => {
        const label = groupLabel(result.kind);
        const prevLabel = i > 0 ? groupLabel(results[i - 1].kind) : null;
        const showLabel = label !== null && label !== prevLabel;
        const shortcutIdx = launchableResults.indexOf(result);
        const showShortcut = shortcutIdx >= 0 && shortcutIdx < 9;
        return (
          <Fragment key={result.id}>
            {showLabel && (
              <div
                ref={i === selectedIndex ? selectedLabelRef : null}
                className={`result-group-label${i === 0 ? " first" : ""}`}
              >
                <span>{label}</span>
              </div>
            )}
            <div
              ref={i === selectedIndex ? selectedRef : null}
              className={`result-row${i === selectedIndex ? " selected" : ""}`}
              style={{ '--row-i': i } as CSSProperties}
              role="option"
              aria-selected={i === selectedIndex}
              onClick={() => {
                onSelect(i);
                // Clicking a running timer selects it (shows preview) but doesn't stop it.
                if (result.kind !== "timer-item") {
                  onLaunch(result);
                }
              }}
            >
              <ResultIcon icon_path={result.icon_path} iconDataUri={result.icon_data_uri} title={result.title} kind={result.kind} />
              <div className="result-text">
                <div className="result-title">{result.title}</div>
                {result.subtitle && <div className="result-subtitle">{shortenPath(result.subtitle)}</div>}
              </div>
              <div className="result-meta">
                {result.kind === "file" && result.file_size != null
                  ? formatBytes(result.file_size)
                  : ""}
              </div>
              <div className="result-shortcut" style={!showShortcut ? { visibility: 'hidden' } : undefined}>
                {showShortcut ? shortcutIdx + 1 : ""}
              </div>
            </div>
          </Fragment>
        );
      })}
    </div>
  );
}
