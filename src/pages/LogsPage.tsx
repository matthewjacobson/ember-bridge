/**
 * Logs page: incremental polling of the app's ring-buffer log.
 */

import { useEffect, useRef, useState } from "react";
import type { LogEntry } from "../api/types";
import { useBridge } from "../hooks/useBridge";
import { EmptyState } from "../components/ui";
import { formatTime } from "../lib/format";

export function LogsPage() {
  const { client } = useBridge();
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const lastSeq = useRef(0);
  const scroller = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!client) return;
    let stopped = false;
    const poll = async () => {
      try {
        const result = await client.logs(lastSeq.current);
        if (stopped || result.entries.length === 0) return;
        lastSeq.current = result.lastSeq;
        setEntries((prev) => [...prev, ...result.entries].slice(-1000));
      } catch {
        // Transient; next poll retries.
      }
    };
    poll();
    const handle = setInterval(poll, 1000);
    return () => {
      stopped = true;
      clearInterval(handle);
    };
  }, [client]);

  // Keep the newest entry in view.
  useEffect(() => {
    scroller.current?.scrollTo({ top: scroller.current.scrollHeight });
  }, [entries]);

  return (
    <div className="page page-logs">
      {entries.length === 0 ? (
        <EmptyState>No activity yet.</EmptyState>
      ) : (
        <div className="log-view" ref={scroller}>
          {entries.map((entry) => (
            <div key={entry.seq} className={`log-entry log-${entry.level}`}>
              <span className="log-time">{formatTime(entry.timestampMs)}</span>
              <span className="log-level">{entry.level}</span>
              <span className="log-message">{entry.message}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
