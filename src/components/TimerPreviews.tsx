import { useState, useEffect } from "react";
import { SearchResult } from "../types";
import { ClockIconLg } from "../icons";

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
        <button className="btn-primary btn-danger" onClick={onStop}>
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
          <button className="btn-primary" onClick={onStart}>
            Start <span className="btn-kbd">↵</span>
          </button>
        </div>
      )}
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
        <button className="btn-primary" onClick={onDismiss}>
          Dismiss <span className="btn-kbd">↵</span>
        </button>
      </div>
    </div>
  );
}
