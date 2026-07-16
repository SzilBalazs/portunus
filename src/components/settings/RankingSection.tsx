import { Config } from "../../types";
import Toggle from "./Toggle";
import Select from "./Select";
import SectionHeader from "./SectionHeader";
import SettingsGroup from "./SettingsGroup";
import SettingsField from "./SettingsField";
import TickSlider from "./TickSlider";
import CategoryList from "./ranking/CategoryList";
import PinsList from "./ranking/PinsList";
import RankingPlayground from "./ranking/RankingPlayground";
import {
  mergedOrder,
  RANKING_DEFAULTS,
  BOOST_STOPS,
  BALANCE_STOPS,
  FADE_PRESETS,
  fadeLabel,
} from "./ranking/categories";

interface Props {
  config: Config;
  onChange: (c: Config) => void;
}

const STRICTNESS_OPTIONS = [
  { label: "Loose",    value: 0.03 },
  { label: "Balanced", value: 0.06 },
  { label: "Strict",   value: 0.12 },
] as const;

function strictnessLabel(v: number): string {
  return STRICTNESS_OPTIONS.find(o => Math.abs(o.value - v) < 0.001)?.label ?? `Custom (${v.toFixed(2)})`;
}

function ResetButton({ onClick }: { onClick: () => void }) {
  return (
    <button type="button" className="settings-group-reset" onClick={onClick}>
      Reset
    </button>
  );
}

export default function RankingSection({ config, onChange }: Props) {
  const setSearch = (patch: Partial<Config["search"]>) =>
    onChange({ ...config, search: { ...config.search, ...patch } });
  const setFrecency = (patch: Partial<Config["frecency"]>) =>
    onChange({ ...config, frecency: { ...config.frecency, ...patch } });
  const setRanking = (patch: Partial<Config["ranking"]>) =>
    onChange({ ...config, ranking: { ...config.ranking, ...patch } });

  const fr = config.frecency;
  const rk = config.ranking;

  const orderDirty =
    mergedOrder(rk.category_order).join() !== RANKING_DEFAULTS.category_order.join() ||
    Object.values(rk.category_weights).some(w => w !== 50) ||
    Object.values(rk.extension_weights).some(w => w !== 50);
  const boostsDirty =
    rk.match_boost.exact !== RANKING_DEFAULTS.match_boost.exact ||
    rk.match_boost.prefix !== RANKING_DEFAULTS.match_boost.prefix ||
    rk.match_boost.word_start !== RANKING_DEFAULTS.match_boost.word_start ||
    Math.abs(config.search.min_quality - 0.06) > 0.001;
  const historyDirty = !fr.enabled || fr.half_life_days !== 14 || rk.match_vs_history !== 50;

  return (
    <div className="settings-section settings-section--ranking">
      <SectionHeader
        title="Ranking"
        desc="Choose what wins when results compete."
      />

      <div className="ranking-layout">
        <div className="ranking-knobs">
      <SettingsGroup
        title="Category priority"
        desc="Higher rows win ties. Drag to reorder; set each weight or hide it."
        action={orderDirty ? (
          <ResetButton onClick={() => setRanking({
            category_order: [...RANKING_DEFAULTS.category_order],
            category_weights: {},
            extension_weights: {},
          })} />
        ) : undefined}
      >
        <CategoryList config={config} setRanking={setRanking} />
      </SettingsGroup>

      <SettingsGroup
        title="Match quality"
        desc="How much a matching title lifts a result."
        action={boostsDirty ? (
          <ResetButton onClick={() => {
            setSearch({ min_quality: 0.06 });
            setRanking({ match_boost: { ...RANKING_DEFAULTS.match_boost } });
          }} />
        ) : undefined}
      >
        <SettingsField
          name="Match strictness"
          desc="How close a match must be. Loose shows more, Strict shows less."
        >
          <Select
            options={STRICTNESS_OPTIONS.map(o => ({ label: o.label }))}
            value={strictnessLabel(config.search.min_quality)}
            onChange={label => {
              const opt = STRICTNESS_OPTIONS.find(o => o.label === label);
              if (opt) setSearch({ min_quality: opt.value });
            }}
          />
        </SettingsField>

        <SettingsField
          name="Exact title match"
          desc="Title equals your query."
        >
          <TickSlider
            label="Exact match"
            stops={BOOST_STOPS.exact}
            value={rk.match_boost.exact}
            onChange={v => setRanking({ match_boost: { ...rk.match_boost, exact: v } })}
          />
        </SettingsField>

        <SettingsField
          name="Title starts with query"
          desc="Title begins with your query."
        >
          <TickSlider
            label="Prefix match"
            stops={BOOST_STOPS.prefix}
            value={rk.match_boost.prefix}
            onChange={v => setRanking({ match_boost: { ...rk.match_boost, prefix: v } })}
          />
        </SettingsField>

        <SettingsField
          name="Word starts with query"
          desc="A word in the title begins with your query."
        >
          <TickSlider
            label="Word start match"
            stops={BOOST_STOPS.word_start}
            value={rk.match_boost.word_start}
            onChange={v => setRanking({ match_boost: { ...rk.match_boost, word_start: v } })}
          />
        </SettingsField>
      </SettingsGroup>

      <SettingsGroup
        title="Launch history"
        desc="Surface things you open often."
        action={historyDirty ? (
          <ResetButton onClick={() => {
            setFrecency({ enabled: true, half_life_days: 14 });
            setRanking({ match_vs_history: 50 });
          }} />
        ) : undefined}
      >
        <SettingsField
          name="Track launch history"
          desc="Promote apps and files you open often."
        >
          <Toggle label="Track launch history" checked={fr.enabled} onChange={v => setFrecency({ enabled: v })} />
        </SettingsField>

        <SettingsField
          name="Match vs history"
          desc="Prefer a close match or your usual pick."
        >
          <TickSlider
            label="Match vs history"
            stops={BALANCE_STOPS}
            value={rk.match_vs_history}
            onChange={v => setRanking({ match_vs_history: v })}
          />
        </SettingsField>

        <SettingsField
          name="History fade"
          desc="How long until an unused item fades away."
        >
          <Select
            options={FADE_PRESETS.map(p => ({ label: p.label }))}
            value={fadeLabel(fr.half_life_days)}
            onChange={label => {
              const p = FADE_PRESETS.find(o => o.label === label);
              if (p) setFrecency({ half_life_days: p.days });
            }}
          />
        </SettingsField>
      </SettingsGroup>

      <SettingsGroup
        title="Pinned results"
        desc="Always first while you type toward their query."
      >
        <PinsList />
      </SettingsGroup>
        </div>

        <aside className="ranking-play">
          <RankingPlayground config={config} />
        </aside>
      </div>
    </div>
  );
}
