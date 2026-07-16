import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Config, ExplainResponse, SearchResult } from "../../../types";
import ResultIcon from "../../ResultIcon";
import ScoreBar from "../ScoreBar";

interface Props {
  config: Config;
}

const MAX_ROWS = 5;
const DEBOUNCE_MS = 200;

/**
 * Live ranking playground: type a test query, watch results re-rank as the
 * knobs above change. Scores against the *staged* config (overrides param) -
 * never the lagging autosaved disk state. Extensions are never executed here.
 */
export default function RankingPlayground({ config }: Props) {
  const [query, setQuery] = useState("");
  const [response, setResponse] = useState<ExplainResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showNumbers, setShowNumbers] = useState(false);
  const [hoverId, setHoverId] = useState<string | null>(null);
  const requestSeq = useRef(0);

  // Movement chips: compare the previous result-id order against the new one.
  const prevOrder = useRef<string[]>([]);
  const [deltas, setDeltas] = useState<Map<string, number>>(new Map());

  // FLIP: glide rows to their new slot when a knob reorders them.
  const rowEls = useRef<Map<string, HTMLElement>>(new Map());
  const rowTops = useRef<Map<string, number>>(new Map());

  const ranking = config.ranking;
  const frecencyEnabled = config.frecency.enabled;

  useEffect(() => {
    const q = query.trim();
    if (!q) {
      requestSeq.current++;
      setResponse(null);
      setError(null);
      return;
    }
    const seq = ++requestSeq.current;
    const t = setTimeout(() => {
      invoke<ExplainResponse>("search_explain", {
        query: q,
        overrides: { ranking, frecency_enabled: frecencyEnabled },
      })
        .then(res => {
          if (requestSeq.current !== seq) return;
          const order = res.results.map(r => r.id);
          const moves = new Map<string, number>();
          if (prevOrder.current.length > 0) {
            for (const [i, id] of order.entries()) {
              const was = prevOrder.current.indexOf(id);
              if (was >= 0 && was !== i) moves.set(id, was - i);
            }
          }
          prevOrder.current = order;
          setDeltas(moves);
          setResponse(res);
          setError(null);
        })
        .catch(e => {
          if (requestSeq.current !== seq) return;
          setError(String(e));
        });
    }, DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [query, ranking, frecencyEnabled]);

  // Alt-held numeric disclosure (house convention).
  useEffect(() => {
    const down = (e: KeyboardEvent) => { if (e.key === "Alt") setShowNumbers(true); };
    const up = (e: KeyboardEvent) => { if (e.key === "Alt") setShowNumbers(false); };
    const blur = () => setShowNumbers(false);
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", blur);
    };
  }, []);

  const results = (response?.results ?? []).slice(0, MAX_ROWS);
  const overflow = (response?.results.length ?? 0) - results.length;
  const maxScore = Math.max(1, ...results.map(r => r.score + (r.breakdown?.penalty ?? 0)));

  useLayoutEffect(() => {
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const next = new Map<string, number>();
    for (const [id, el] of rowEls.current) {
      const top = el.offsetTop;
      next.set(id, top);
      if (reduced) continue;
      const prev = rowTops.current.get(id);
      if (prev === undefined || Math.abs(prev - top) < 0.5) continue;
      el.animate(
        [{ transform: `translateY(${prev - top}px)` }, { transform: "translateY(0)" }],
        { duration: 180, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
      );
    }
    rowTops.current = next;
  }, [results]);

  const active = query.trim().length > 0;

  return (
    <div className={`settings-playground${active ? " active" : ""}`} data-show-scores={showNumbers || undefined}>
      <div className="settings-playground-input-row">
        <input
          className="settings-playground-input"
          type="text"
          value={query}
          spellCheck={false}
          placeholder="Try a query like code or invoice"
          onChange={e => setQuery(e.target.value)}
        />
      </div>
      {active && results.length > 0 && (
        <div className="settings-playground-hint">hold Alt for numbers</div>
      )}
      {active && (
        <div className="settings-playground-body">
          {results.length > 0 && (
            <div className="settings-playground-legend" aria-hidden="true">
              <span data-seg="base">base</span>
              <span data-seg="match">match</span>
              <span data-seg="frecency">history</span>
              <span data-seg="pin">pin</span>
            </div>
          )}
          {results.map(r => (
            <PlaygroundRow
              key={r.id}
              result={r}
              max={maxScore}
              delta={deltas.get(r.id) ?? 0}
              showNumbers={showNumbers || hoverId === r.id}
              rowRef={el => {
                if (el) rowEls.current.set(r.id, el);
                else rowEls.current.delete(r.id);
              }}
              onHover={h => setHoverId(h ? r.id : null)}
            />
          ))}
          {overflow > 0 && <div className="settings-playground-more">+{overflow} more</div>}
          {response && results.length === 0 && !error && (
            <div className="settings-playground-empty">No results for this query</div>
          )}
          {error && <div className="settings-playground-error">{error}</div>}
          {response && response.skipped_extensions.length > 0 && (
            <div className="settings-playground-note">
              Extension results aren&apos;t scored in the playground ({response.skipped_extensions.join(", ")}).
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function PlaygroundRow({
  result,
  max,
  delta,
  showNumbers,
  rowRef,
  onHover,
}: {
  result: SearchResult;
  max: number;
  delta: number;
  showNumbers: boolean;
  rowRef: (el: HTMLElement | null) => void;
  onHover: (hovering: boolean) => void;
}) {
  return (
    <div
      className="settings-playground-row"
      ref={rowRef}
      onMouseEnter={() => onHover(true)}
      onMouseLeave={() => onHover(false)}
    >
      <div className="settings-playground-row-head">
        <ResultIcon
          icon_path={result.icon_path}
          iconDataUri={result.icon_data_uri}
          glyph={result.command?.glyph}
          title={result.title}
          kind={result.kind}
        />
        <div className="settings-playground-row-text">
          <div className="settings-playground-row-title">
            {result.title}
            {result.pinned && <span className="settings-playground-pin">★</span>}
          </div>
          {result.subtitle && <div className="settings-playground-row-sub">{result.subtitle}</div>}
        </div>
        <span
          className={`settings-playground-delta${delta > 0 ? " up" : delta < 0 ? " down" : ""}`}
          // Re-mount per change so the fade-out animation restarts.
          key={`${result.id}:${delta}`}
        >
          {delta > 0 ? `▲${delta}` : delta < 0 ? `▼${-delta}` : ""}
        </span>
      </div>
      {result.breakdown && (
        <div className="settings-playground-score">
          <ScoreBar breakdown={result.breakdown} max={max} showNumbers={showNumbers} />
        </div>
      )}
    </div>
  );
}
