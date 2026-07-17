import type { ReactNode } from "react";

interface Props {
  /** Heading, rendered after a ⚠ glyph. */
  title: string;
  /** Explanatory body (why this is dangerous). */
  children: ReactNode;
  /** Text next to the mandatory acknowledgement checkbox. */
  ackLabel: string;
  acked: boolean;
  onAckChange: (v: boolean) => void;
}

/**
 * Shared shell for a sandbox-relaxing consent warning: the red danger box, a
 * ⚠ title, a body, and the mandatory "I understand" checkbox. Concrete notices
 * (spawn, any-host network) supply their own title/body/label so every hard
 * gate looks and behaves identically.
 */
export default function DangerNotice({ title, children, ackLabel, acked, onAckChange }: Props) {
  return (
    <div className="settings-ext-danger">
      <div className="settings-ext-danger-title">⚠ {title}</div>
      {children}
      <label className="settings-ext-danger-ack">
        <input type="checkbox" checked={acked} onChange={e => onAckChange(e.target.checked)} />
        {ackLabel}
      </label>
    </div>
  );
}
