/**
 * App shell: sidebar navigation + page router (plain state, no router
 * dependency — five static pages don't warrant one).
 */

import { useEffect, useState } from "react";
import { BridgeProvider, useBridge } from "./hooks/useBridge";
import { usePolling } from "./hooks/usePolling";
import type { BridgeStatus, PendingPairing } from "./api/types";
import type { BridgeClient } from "./api/client";
import { listDongles } from "./api/dongle";
import emberIcon from "./assets/ember-icon.svg";
import { MachinesPage } from "./pages/MachinesPage";
import { SetupPage } from "./pages/SetupPage";
import { SendPage } from "./pages/SendPage";
import { LogsPage } from "./pages/LogsPage";
import { SettingsPage } from "./pages/SettingsPage";
import "./App.css";

type Page = "machines" | "setup" | "send" | "logs" | "settings";

const PAGES: { id: Page; label: string }[] = [
  { id: "machines", label: "Machines" },
  { id: "send", label: "Send" },
  { id: "logs", label: "Logs" },
  { id: "settings", label: "Settings" },
];

/**
 * Approve/Deny prompt for a browser's pairing request. Rendered above every
 * page — a pairing request should be impossible to miss, and approval must
 * live in this window precisely because no web page can reach into it.
 */
function PairingBanner({ client }: { client: BridgeClient }) {
  const pending = usePolling<PendingPairing | null>(
    () => client.pairingPending(),
    2000,
  );
  const [busy, setBusy] = useState(false);

  if (!pending.data) return null;
  const request = pending.data;

  const respond = async (approve: boolean) => {
    setBusy(true);
    try {
      await client.respondPairing(request.id, approve);
    } catch {
      // Request expired or was already answered; the next poll clears it.
    } finally {
      setBusy(false);
      void pending.refresh();
    }
  };

  return (
    <div className="pairing-banner">
      <div className="pairing-text">
        <strong>{request.origin}</strong>
        {request.appName !== "Unnamed app" && ` (${request.appName})`} wants to
        connect to your embroidery machines.
      </div>
      <div className="pairing-actions">
        <button className="danger" disabled={busy} onClick={() => respond(false)}>
          Deny
        </button>
        <button className="primary" disabled={busy} onClick={() => respond(true)}>
          Approve
        </button>
      </div>
    </div>
  );
}

function Shell() {
  const { client, connectError, selectedIp } = useBridge();
  const [page, setPage] = useState<Page>("machines");
  const status = usePolling<BridgeStatus>(
    client ? () => client.status() : null,
    5000,
  );

  // The setup entry only exists while a dongle is physically plugged into
  // this computer (cheap USB enumeration, no port is opened).
  const dongles = usePolling(listDongles, 3000);
  const donglePresent = (dongles.data?.length ?? 0) > 0;
  useEffect(() => {
    // Unplugged while on the (now hidden) setup page: don't strand the user.
    if (page === "setup" && !donglePresent) setPage("machines");
  }, [page, donglePresent]);

  if (connectError) {
    return (
      <div className="boot-error">
        <h1>Ember Bridge</h1>
        <p>Could not reach the app backend: {connectError}</p>
      </div>
    );
  }

  return (
    <div className="shell">
      <nav className="sidebar">
        <div className="brand">
          <img className="brand-mark" src={emberIcon} alt="" /> Ember Bridge
        </div>
        {PAGES.map((p) => (
          <button
            key={p.id}
            className={`nav-item ${page === p.id ? "active" : ""}`}
            onClick={() => setPage(p.id)}
          >
            {p.label}
            {p.id === "send" && (status.data?.pendingUploads ?? 0) > 0 && (
              <span className="badge">{status.data!.pendingUploads}</span>
            )}
          </button>
        ))}
        {donglePresent && (
          <button
            className={`nav-item setup-cta ${page === "setup" ? "active" : ""}`}
            onClick={() => setPage("setup")}
          >
            Ember Connect Set Up
          </button>
        )}
        <div className="sidebar-footer">
          <div className={`dot ${status.data?.server.running ? "dot-ok" : "dot-err"}`} />
          {status.data?.server.running
            ? `API on :${status.data.server.port}`
            : "API offline"}
          {selectedIp && <div className="dim">Target: {selectedIp}</div>}
        </div>
      </nav>
      <main className="content">
        {client && <PairingBanner client={client} />}
        {page === "machines" && <MachinesPage />}
        {page === "setup" && <SetupPage />}
        {page === "send" && <SendPage />}
        {page === "logs" && <LogsPage />}
        {page === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}

export default function App() {
  return (
    <BridgeProvider>
      <Shell />
    </BridgeProvider>
  );
}
