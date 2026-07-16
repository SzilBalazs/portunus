import { ReactNode } from "react";
import { Config } from "../../../types";
import { useExtensionMeta } from "../../../extensions/meta";
import SortableList from "../SortableList";
import Badge from "../Badge";
import WeightPicker from "./WeightPicker";
import { categoryMeta, mergedOrder } from "./categories";

interface Props {
  config: Config;
  setRanking: (patch: Partial<Config["ranking"]>) => void;
}

const GripIcon = () => (
  <svg width="10" height="14" viewBox="0 0 10 14" fill="currentColor" aria-hidden="true">
    {[1, 7].map(x => [2, 7, 12].map(y => <circle key={`${x}${y}`} cx={x + 1} cy={y} r="1.3" />))}
  </svg>
);

const ChevronIcon = ({ open }: { open: boolean }) => (
  <svg
    width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor"
    strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"
    style={{ transform: open ? "rotate(90deg)" : undefined, transition: "transform 0.15s" }}
  >
    <path d="M4 2l4 4-4 4" />
  </svg>
);

/** A glyph per category, drawn in the app's own icon language so the ladder
 *  reads like the sidebar and the result icons in the live playground. */
const svg = (children: ReactNode) => (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
    strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    {children}
  </svg>
);
const CATEGORY_GLYPHS: Record<string, ReactNode> = {
  calc: svg(<>
    <rect x="4" y="2" width="16" height="20" rx="2" /><line x1="8" y1="6" x2="16" y2="6" />
    <line x1="8" y1="11" x2="8.01" y2="11" /><line x1="12" y1="11" x2="12.01" y2="11" /><line x1="16" y1="11" x2="16.01" y2="11" />
    <line x1="8" y1="16" x2="8.01" y2="16" /><line x1="12" y1="16" x2="12.01" y2="16" /><line x1="16" y1="16" x2="16.01" y2="16" />
  </>),
  app: svg(<>
    <rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" />
    <rect x="14" y="14" width="7" height="7" /><rect x="3" y="14" width="7" height="7" />
  </>),
  command: svg(<>
    <rect x="3" y="4" width="18" height="16" rx="2" /><polyline points="7 9 10 12 7 15" /><line x1="13" y1="15" x2="17" y2="15" />
  </>),
  extension: svg(
    <path d="M20.5 11H19V7a2 2 0 0 0-2-2h-4V3.5a2.5 2.5 0 0 0-5 0V5H4a2 2 0 0 0-2 2v3.8h1.5a2.7 2.7 0 0 1 0 5.4H2V20a2 2 0 0 0 2 2h3.8v-1.5a2.7 2.7 0 0 1 5.4 0V22H17a2 2 0 0 0 2-2v-4h1.5a2.5 2.5 0 0 0 0-5z" />
  ),
  file: svg(<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />),
  dict: svg(<>
    <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" /><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z" />
  </>),
};

/**
 * The ranking centerpiece: a slim priority ladder. Drag to reorder (top wins
 * ties); set each category's weight inline. Extensions expand to reveal a
 * per-extension weight for every installed extension.
 */
export default function CategoryList({ config, setRanking }: Props) {
  const ranking = config.ranking;
  const order = mergedOrder(ranking.category_order);
  const extensions = useExtensionMeta().filter(e => e.enabled);

  const weightOf = (key: string) => ranking.category_weights[key] ?? 50;
  const setWeight = (key: string, w: number) =>
    setRanking({ category_weights: { ...ranking.category_weights, [key]: w } });
  const extWeightOf = (name: string) => ranking.extension_weights[name] ?? 50;
  const setExtWeight = (name: string, w: number) =>
    setRanking({ extension_weights: { ...ranking.extension_weights, [name]: w } });

  const anyHidden = order.some(k => weightOf(k) === 0);

  return (
    <>
    <SortableList
      className="category-ladder"
      items={order}
      getKey={k => k}
      ariaLabel="Result category priority"
      onReorder={keys => setRanking({ category_order: keys })}
      renderRow={(key, ctx) => {
        const meta = categoryMeta(key)!;
        const expandable = key === "extension";
        return (
          <>
            <span className="settings-sortable-grip" {...ctx.handleProps} aria-label={`Reorder ${meta.label}`}>
              <GripIcon />
            </span>
            <span className="category-rank" aria-hidden="true">{order.indexOf(key) + 1}</span>
            <span className="category-glyph" aria-hidden="true">{CATEGORY_GLYPHS[key]}</span>
            <div className="category-name" title={meta.desc}>{meta.label}</div>
            <WeightPicker label={meta.label} value={weightOf(key)} onChange={v => setWeight(key, v)} />
            {expandable ? (
              <button
                type="button"
                className="settings-sortable-chevron"
                aria-label={`${ctx.expanded ? "Collapse" : "Expand"} per-extension weights`}
                aria-expanded={ctx.expanded}
                onClick={ctx.toggleExpand}
              >
                <ChevronIcon open={ctx.expanded} />
              </button>
            ) : (
              <span className="settings-sortable-chevron-spacer" aria-hidden="true" />
            )}
          </>
        );
      }}
      renderExpanded={key => {
        if (key !== "extension") return null;
        if (extensions.length === 0) {
          return <div className="settings-sortable-detail-body">
            <div className="settings-sortable-detail-empty">No extensions installed.</div>
          </div>;
        }
        return (
          <div className="settings-sortable-detail-body">
            {extensions.map(ext => (
              <div className="settings-sortable-detail-row" key={ext.name}>
                <span className="settings-sortable-detail-label">
                  {ext.name}
                  {ext.dev && <Badge tone="dev">dev</Badge>}
                </span>
                <WeightPicker label={ext.name} value={extWeightOf(ext.name)} onChange={v => setExtWeight(ext.name, v)} />
              </div>
            ))}
          </div>
        );
      }}
    />
    {anyHidden && (
      <div className="category-ladder-note">
        Hidden categories leave the main results but stay reachable in scoped searches.
      </div>
    )}
    </>
  );
}
