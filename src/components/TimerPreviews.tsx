import { useState, useEffect } from "react";
import { SearchResult } from "../types";
import { ClockIconLg, EnterIcon } from "../icons";

const TIMER_HINT_STYLES = `
.timer-hint {
  padding: 20px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  flex: 1;
}
.timer-hint-hero { display: flex; align-items: center; gap: 14px; }
.timer-hint-name { font-size: 15px; font-weight: 600; color: var(--fg); letter-spacing: -0.01em; }
.timer-hint-desc { margin-top: 4px; font-size: 12px; color: var(--fg-mute); line-height: 1.55; }
.timer-hint-divider { height: 1px; background: var(--line-soft); }
.timer-hint-prose { font-size: 12px; color: var(--fg-mute); line-height: 1.7; }
.timer-hint-token {
  font: 500 11.5px/1 "JetBrains Mono","Fira Code",monospace;
  color: var(--accent); background: var(--accent-soft);
  padding: 2px 5px; border-radius: 3px;
}
.timer-hint-example {
  display: flex; align-items: baseline; gap: 7px;
  padding: 9px 12px; background: #15120f;
  border-radius: var(--radius-sm); border-left: 2px solid var(--accent);
}
.timer-hint-ex-cmd { font: 600 13px/1 "JetBrains Mono","Fira Code",monospace; color: var(--accent); }
.timer-hint-ex-word { font: 400 13px/1 "JetBrains Mono","Fira Code",monospace; color: var(--fg-mute); }
.timer-hint-chips { display: flex; gap: 5px; flex-wrap: wrap; }
.timer-hint-chip-tag {
  font: 500 9px/1 "JetBrains Mono","Fira Code",monospace;
  color: var(--accent); background: var(--accent-soft);
  border: 1px solid rgba(214,163,112,0.2); padding: 3px 7px; border-radius: 4px; white-space: nowrap;
}
`;

if (typeof document !== 'undefined') {
  const id = 'timer-hint-styles';
  if (!document.getElementById(id)) {
    const el = document.createElement('style');
    el.id = id;
    el.textContent = TIMER_HINT_STYLES;
    document.head.appendChild(el);
  }
}

function TimerDisplay({ secs }: { secs: number }) {
  const s = Math.max(0, Math.floor(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return <>{h}<span className="timer-unit">h</span> {m}<span className="timer-unit">m</span> {sec}<span className="timer-unit">s</span></>;
  if (m > 0) return <>{m}<span className="timer-unit">m</span> {sec}<span className="timer-unit">s</span></>;
  return <>{sec}<span className="timer-unit">s</span></>;
}

// ── running timer ─────────────────────────────────────────────────────────────

interface TimerPreviewProps {
  result: SearchResult;
  onStop: () => void;
}

export function TimerPreview({ result, onStop }: TimerPreviewProps) {
  const [now, setNow] = useState(() => Date.now() / 1000);

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now() / 1000), 100);
    return () => clearInterval(id);
  }, []);

  const startedAt = result.created ?? 0;
  const durationSecs = result.file_size ?? 0;
  const elapsed = now - startedAt;
  const elapsedInt = Math.floor(elapsed);
  const remaining = Math.max(0, durationSecs - elapsed);
  const progress = durationSecs > 0 ? Math.min(1, elapsed / durationSecs) : 1;
  const isDone = remaining <= 0;

  const label = result.title;
  const startedStr =
    elapsedInt < 60 ? `${elapsedInt}s ago`
    : elapsedInt < 3600 ? `${Math.floor(elapsedInt / 60)}m ${elapsedInt % 60}s ago`
    : `${Math.floor(elapsedInt / 3600)}h ago`;

  return (
    <div className="timer-preview">
      <div className="timer-preview-hero">
        <div className="timer-preview-icon-wrap"><ClockIconLg /></div>
        <div>
          <div className="timer-preview-name">{label}</div>
          <div className="timer-preview-hint">Started {startedStr}</div>
        </div>
      </div>
      <div className={`timer-remaining${isDone ? " done" : ""}`}>
        {isDone ? "Done" : <TimerDisplay secs={remaining} />}
      </div>
      <div className="timer-progress-track">
        <div className="timer-progress-fill" style={{ width: `${progress * 100}%` }} />
      </div>
      <div className="timer-preview-actions">
        <button className="btn-primary btn-danger" onClick={onStop} tabIndex={-1}>
          Stop timer <span className="btn-kbd">Del</span>
        </button>
      </div>
    </div>
  );
}

// ── create timer ──────────────────────────────────────────────────────────────

interface TimerCreatePreviewProps {
  result: SearchResult;
  onStart: () => void;
}

export function TimerCreatePreview({ result, onStart }: TimerCreatePreviewProps) {
  const durationSecs = result.file_size;
  const hasAction = !!result.exec;
  const hasLabel = hasAction && result.subtitle !== "↵ to start";

  return (
    <div className="timer-preview">
      <div className="timer-preview-hero">
        <div className="timer-preview-icon-wrap"><ClockIconLg /></div>
        <div>
          <div className="timer-preview-name">
            {hasAction ? result.title : "Start a timer"}
          </div>
          <div className="timer-preview-hint">
            {hasAction
              ? (hasLabel ? result.subtitle : "Ready to start")
              : "30s · 5m · 1h30m"}
          </div>
        </div>
      </div>
      {durationSecs != null && (
        <div className="timer-remaining"><TimerDisplay secs={durationSecs} /></div>
      )}
      {hasAction && (
        <div className="timer-preview-actions">
          <button className="btn-primary" onClick={onStart} tabIndex={-1}>
            Start <span className="btn-kbd"><EnterIcon /></span>
          </button>
        </div>
      )}
    </div>
  );
}

// ── timer hint (discovery panel) ─────────────────────────────────────────────

export function TimerHintPreview() {
  return (
    <div className="timer-hint">
      <div className="timer-hint-hero">
        <div className="file-preview-icon-wrap">
          <ClockIconLg />
        </div>
        <div>
          <div className="timer-hint-name">Countdown Timer</div>
          <div className="timer-hint-desc">
            Start a timer with any duration. Get a chime when it finishes.
          </div>
        </div>
      </div>
      <div className="timer-hint-divider" />
      <div className="timer-hint-prose">
        Type{' '}
        <span className="timer-hint-token">timer</span>
        {' '}followed by a duration:
      </div>
      <div className="timer-hint-example">
        <span className="timer-hint-ex-cmd">timer</span>
        <span className="timer-hint-ex-word">30m workout</span>
      </div>
      <div className="timer-hint-chips">
        {['30s', '5m', '1h', '1h30m'].map(chip => (
          <span key={chip} className="timer-hint-chip-tag">{chip}</span>
        ))}
      </div>
    </div>
  );
}

// ── expired timer ─────────────────────────────────────────────────────────────

interface TimerExpiredPreviewProps {
  label: string;
  onDismiss: () => void;
}

export function TimerExpiredPreview({ label, onDismiss }: TimerExpiredPreviewProps) {
  return (
    <div className="timer-preview">
      <div className="timer-preview-hero">
        <div className="timer-preview-icon-wrap"><ClockIconLg /></div>
        <div>
          <div className="timer-preview-name">{label}</div>
          <div className="timer-preview-hint">Timer finished</div>
        </div>
      </div>
      <div className="timer-remaining done">Done</div>
      <div className="timer-progress-track">
        <div className="timer-progress-fill" style={{ width: "100%" }} />
      </div>
      <div className="timer-preview-actions">
        <button className="btn-primary" onClick={onDismiss} tabIndex={-1}>
          Dismiss <span className="btn-kbd"><EnterIcon /></span>
        </button>
      </div>
    </div>
  );
}
