/**
 * App shell: sidebar navigation + page router (plain state, no router
 * dependency — four static pages don't warrant one).
 */

import { useState } from "react";
import { BridgeProvider, useBridge } from "./hooks/useBridge";
import { usePolling } from "./hooks/usePolling";
import type { BridgeStatus } from "./api/types";
import { MachinesPage } from "./pages/MachinesPage";
import { SendPage } from "./pages/SendPage";
import { LogsPage } from "./pages/LogsPage";
import { SettingsPage } from "./pages/SettingsPage";
import "./App.css";

type Page = "machines" | "send" | "logs" | "settings";

const PAGES: { id: Page; label: string }[] = [
  { id: "machines", label: "Machines" },
  { id: "send", label: "Send" },
  { id: "logs", label: "Logs" },
  { id: "settings", label: "Settings" },
];

function Shell() {
  const { client, connectError, selectedIp } = useBridge();
  const [page, setPage] = useState<Page>("machines");
  const status = usePolling<BridgeStatus>(
    client ? () => client.status() : null,
    5000,
  );

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
          <span className="brand-mark">✦</span> Ember Bridge
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
        <div className="sidebar-footer">
          <div className={`dot ${status.data?.server.running ? "dot-ok" : "dot-err"}`} />
          {status.data?.server.running
            ? `API on :${status.data.server.port}`
            : "API offline"}
          {selectedIp && <div className="dim">Target: {selectedIp}</div>}
        </div>
      </nav>
      <main className="content">
        {page === "machines" && <MachinesPage />}
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
