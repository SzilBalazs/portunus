import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ExtensionLogEntry } from "../../../types";

interface Props {
  extension: string;
}

const POLL_MS = 2000;

function fmtTime(tsMs: number): string {
  return new Date(tsMs).toLocaleTimeString("en-GB", { hour12: false });
}

/**
 * Tail of one extension's log ring buffer. Polls while mounted (the panel is
 * only mounted while its card is expanded) - the ring buffer makes polling
 * idempotent and cheap, and avoids event-bus spam during background refreshes.
 */
export default function ExtensionLogs({ extension }: Props) {
  const [entries, setEntries] = useState<ExtensionLogEntry[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);

  const refresh = useCallback(() => {
    invoke<ExtensionLogEntry[]>("get_extension_logs", { name: extension, limit: 100 })
      .then(setEntries)
      .catch(() => {});
  }, [extension]);

  useEffect(() => {
    refresh();
    const t = window.setInterval(refresh, POLL_MS);
    return () => window.clearInterval(t);
  }, [refresh]);

  // Stick to the bottom unless the user scrolled up to read history.
  useEffect(() => {
    const el = scrollRef.current;
    if (el && stickRef.current) el.scrollTop = el.scrollHeight;
  }, [entries]);

  return (
    <div className="settings-ext-logs">
      <div className="settings-ext-logs-bar">
        <span className="settings-ext-logs-title">Logs</span>
        <button className="settings-btn-secondary" onClick={refresh}>Refresh</button>
      </div>
      <div
        className="settings-ext-logs-list"
        ref={scrollRef}
        onScroll={e => {
          const el = e.currentTarget;
          stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 8;
        }}
      >
        {entries.length === 0 && <div className="settings-ext-logs-empty">No log output yet.</div>}
        {entries.map((e, i) => (
          <div key={`${e.ts_ms}-${i}`} className={`settings-ext-log-row settings-ext-log--${e.level}`}>
            <span className="settings-ext-log-time">{fmtTime(e.ts_ms)}</span>
            <span className="settings-ext-log-msg">{e.message}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
