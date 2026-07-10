/**
 * Poll an async producer on an interval, keeping the latest value.
 * The bridge API is cheap on loopback, so simple polling beats the
 * complexity of a push channel for this app's needs.
 */

import { useCallback, useEffect, useRef, useState } from "react";

export interface Polled<T> {
  data: T | null;
  error: string | null;
  /** Re-run immediately (e.g. after a mutation). */
  refresh: () => Promise<void>;
}

export function usePolling<T>(
  producer: (() => Promise<T>) | null,
  intervalMs: number,
): Polled<T> {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const producerRef = useRef(producer);
  producerRef.current = producer;

  const tick = useCallback(async () => {
    const produce = producerRef.current;
    if (!produce) return;
    try {
      setData(await produce());
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    if (!producer) return;
    let stopped = false;
    const run = () => void tick();
    run();
    const handle = setInterval(() => !stopped && run(), intervalMs);
    return () => {
      stopped = true;
      clearInterval(handle);
    };
    // Restart when the producer's identity flips between null/non-null.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [producer === null, intervalMs, tick]);

  return { data, error, refresh: tick };
}
