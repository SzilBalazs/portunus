export interface ExtTab {
  key: string;
  label: string;
  /** Small trailing count (commands, settings). */
  count?: number;
  /** Attention marker (a permission needing review). */
  alert?: boolean;
}

interface Props {
  tabs: ExtTab[];
  active: string;
  onSelect: (key: string) => void;
}

/**
 * In-card tab strip for the sectioned extension card. Chunks the detail
 * (Overview / Commands / Permissions / Settings / Logs) so nothing dumps at
 * once when a card expands.
 */
export default function ExtCardTabs({ tabs, active, onSelect }: Props) {
  return (
    <div className="settings-ext-tabs" role="tablist">
      {tabs.map(t => (
        <button
          key={t.key}
          type="button"
          role="tab"
          aria-selected={t.key === active}
          className={`settings-ext-tab${t.key === active ? " active" : ""}`}
          onClick={() => onSelect(t.key)}
        >
          {t.label}
          {t.count != null && <span className="settings-ext-tab-count">{t.count}</span>}
          {t.alert && <span className="settings-ext-tab-alert" aria-hidden>!</span>}
        </button>
      ))}
    </div>
  );
}
