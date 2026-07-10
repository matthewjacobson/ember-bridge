/**
 * App-wide context: the connected BridgeClient plus the currently selected
 * machine (shared between the Machines and Send pages).
 */

import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { BridgeClient } from "../api/client";

interface BridgeContextValue {
  client: BridgeClient | null;
  /** Non-null when the Tauri command itself failed. */
  connectError: string | null;
  selectedIp: string | null;
  setSelectedIp: (ip: string | null) => void;
}

const BridgeContext = createContext<BridgeContextValue | null>(null);

export function BridgeProvider({ children }: { children: ReactNode }) {
  const [client, setClient] = useState<BridgeClient | null>(null);
  const [connectError, setConnectError] = useState<string | null>(null);
  const [selectedIp, setSelectedIp] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    BridgeClient.connect()
      .then((c) => !cancelled && setClient(c))
      .catch((e) => !cancelled && setConnectError(String(e)));
    return () => {
      cancelled = true;
    };
  }, []);

  const value = useMemo(
    () => ({ client, connectError, selectedIp, setSelectedIp }),
    [client, connectError, selectedIp],
  );

  return <BridgeContext.Provider value={value}>{children}</BridgeContext.Provider>;
}

export function useBridge(): BridgeContextValue {
  const value = useContext(BridgeContext);
  if (!value) throw new Error("useBridge must be used inside <BridgeProvider>");
  return value;
}
