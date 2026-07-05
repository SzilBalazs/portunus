import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ExtensionLogEntry } from "../../../types";
import { useTauriListener } from "../../../hooks/useTauriListener";

interface Props {
  extension: string;
}

/** Trailing debounce for event-driven refreshes - a chatty extension emits
 * many log lines per call; one refetch per burst is enough. */
const EVENT_DEBOUNCE_MS = 120;
/** Slow fallback poll in case an event is ever dropped. */
const FALLBACK_POLL_MS = 10_000;

function fmtTime(tsMs: number): string {
  return new Date(tsMs).toLocaleTimeString("en-GB", { hour12: false });
}

/**
 * Tail of one extension's log ring buffer. Refreshes on `extension-log`
 * events (pushed by the backend on every log line) with a slow fallback poll -
 * the ring buffer makes refetching idempotent and cheap.
 */
export default function ExtensionLogs({ extension }: Props) {
  const [entries, setEntries] = useState<ExtensionLogEntry[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);
  const debounceRef = useRef<number | undefined>(undefined);

  const refresh = useCallback(() => {
    invoke<ExtensionLogEntry[]>("get_extension_logs", { name: extension, limit: 100 })
      .then(setEntries)
      .catch(() => {});
  }, [extension]);

  useEffect(() => {
    refresh();
    const t = window.setInterval(refresh, FALLBACK_POLL_MS);
    return () => {
      window.clearInterval(t);
      window.clearTimeout(debounceRef.current);
    };
  }, [refresh]);

  useTauriListener<string>("extension-log", name => {
    if (name !== extension) return;
    window.clearTimeout(debounceRef.current);
    debounceRef.current = window.setTimeout(refresh, EVENT_DEBOUNCE_MS);
  }, [extension, refresh]);

  const clear = () => {
    invoke("clear_extension_logs", { name: extension })
      .then(() => setEntries([]))
      .catch(() => {});
  };

  // Stick to the bottom unless the user scrolled up to read history.
  useEffect(() => {
    const el = scrollRef.current;
    if (el && stickRef.current) el.scrollTop = el.scrollHeight;
  }, [entries]);

  return (
    <div className="settings-ext-logs">
      <div className="settings-ext-logs-bar">
        <button className="settings-btn-secondary" onClick={clear}>Clear</button>
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
