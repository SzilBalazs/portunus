import { Fragment, useEffect, useRef } from "react";
import { SearchResult } from "../types";
import { groupLabel, formatBytes, shortenPath } from "../utils";
import ResultIcon from "./ResultIcon";

function Snippet({ text }: { text: string }) {
  // Guarantee the first highlighted term is visible by trimming the prefix
  // to at most ~18 chars. Without this, CSS overflow clips the match away.
  const firstMark = text.indexOf('\x02');
  let display = text;
  if (firstMark > 18) {
    const prefix = text.slice(0, firstMark);
    const cutAt = prefix.lastIndexOf(' ', firstMark - 10);
    display = '…' + text.slice(cutAt === -1 ? firstMark : cutAt + 1);
  }

  const parts = display.split('\x02');
  return (
    <div className="result-snippet">
      {parts.map((part, i) => {
        if (i === 0) return <span key={i}>{part}</span>;
        const j = part.indexOf('\x03');
        if (j === -1) return <span key={i} className="snippet-hl">{part}</span>;
        return (
          <span key={i}>
            <span className="snippet-hl">{part.slice(0, j)}</span>
            {part.slice(j + 1)}
          </span>
        );
      })}
    </div>
  );
}

interface Props {
  results: SearchResult[];
  selectedIndex: number;
  query: string;
  onSelect: (index: number) => void;
  onLaunch: (result?: SearchResult) => void;
  launchableResults: SearchResult[];
}

export default function ResultsList({ results, selectedIndex, query, onSelect, onLaunch, launchableResults }: Props) {
  const selectedRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  return (
    <div className="results-col" role="listbox">
      {query.trim() && results.length === 0 && (
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
                ref={i === selectedIndex ? selectedRef : null}
                className={`result-group-label${i === 0 ? " first" : ""}`}
              >
                <span>{label}</span>
              </div>
            )}
            <div
              ref={i === selectedIndex && !showLabel ? selectedRef : null}
              className={`result-row${i === selectedIndex ? " selected" : ""}`}
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
              <ResultIcon icon_path={result.icon_path} title={result.title} kind={result.kind} />
              <div className="result-text">
                <div className="result-title">{result.title}</div>
                {result.snippet
                  ? <Snippet text={result.snippet} />
                  : result.subtitle && <div className="result-subtitle">{shortenPath(result.subtitle)}</div>
                }
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
