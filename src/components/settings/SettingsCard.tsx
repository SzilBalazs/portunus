import { ReactNode } from "react";

interface Props {
  /** Group label - rendered mono, uppercase, accent. */
  label: ReactNode;
  /** Optional qualifier badge next to the label (e.g. "Extension"). */
  sub?: ReactNode;
  /** Optional count shown after the label. */
  count?: number;
  /** Optional right-aligned control in the header bar (e.g. "Reset all"). */
  action?: ReactNode;
  children: ReactNode;
}

/**
 * A bordered card with a header bar: mono accent label, optional qualifier
 * badge + count, and a right-aligned action. Groups a list of rows under one
 * legible heading. Shared so any section can present the same card shape.
 */
export default function SettingsCard({ label, sub, count, action, children }: Props) {
  return (
    <div className="settings-gcard">
      <div className="settings-gcard-head">
        <div className="settings-gcard-headmain">
          <span className="settings-gcard-label">{label}</span>
          {sub && <span className="settings-gcard-sub">{sub}</span>}
          {count != null && <span className="settings-gcard-count">{count}</span>}
        </div>
        {action}
      </div>
      <div className="settings-gcard-body">{children}</div>
    </div>
  );
}
