import { ReactNode } from "react";

interface Props {
  /** Field title. */
  name: ReactNode;
  /** Supporting description shown under the title. */
  desc?: ReactNode;
  /** Inline warning (e.g. a missing-dependency notice) rendered under the desc. */
  warn?: ReactNode;
  /** Optional stable id (anchoring / tests). */
  id?: string;
  /** Stack the control full-width below the label instead of beside it.
   *  For wide controls (text/secret inputs) that overflow the two-column row. */
  stacked?: boolean;
  /** The control on the right (Toggle, Slider, Select, NumberStepper, …). */
  children: ReactNode;
}

/**
 * The one canonical settings row: title + desc + optional warning on the left,
 * a control slot on the right. Replaces the hand-rolled
 * `settings-field / -label / -name / -desc / -control` block every section used
 * to copy. Compose it with any control primitive.
 */
export default function SettingsField({ name, desc, warn, id, stacked, children }: Props) {
  return (
    <div className={`settings-field${stacked ? " settings-field--stacked" : ""}`} id={id}>
      <div className="settings-field-label">
        <div className="settings-field-name">{name}</div>
        {desc && <div className="settings-field-desc">{desc}</div>}
        {warn}
      </div>
      <div className="settings-field-control">{children}</div>
    </div>
  );
}
