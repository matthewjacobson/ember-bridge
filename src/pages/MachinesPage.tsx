/**
 * Machines page: discovery, manual add, nicknames, selection, and a
 * "test connection" flow that identifies the machine and shows its
 * capabilities.
 */

import { useState } from "react";
import type { MachineInfo, MachinesResponse } from "../api/types";
import { useBridge } from "../hooks/useBridge";
import { usePolling } from "../hooks/usePolling";
import { EmptyState, ErrorNote, Pill, Section } from "../components/ui";
import { formatBytes, machineLabel } from "../lib/format";

export function MachinesPage() {
  const { client, selectedIp, setSelectedIp } = useBridge();
  const machines = usePolling<MachinesResponse>(
    client ? () => client.machines() : null,
    3000,
  );

  const [scanning, setScanning] = useState(false);
  const [manualIp, setManualIp] = useState("");
  const [manualNickname, setManualNickname] = useState("");
  const [actionError, setActionError] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<MachineInfo | null>(null);
  const [testingIp, setTestingIp] = useState<string | null>(null);

  if (!client) return null;

  const saved = machines.data?.saved ?? [];
  const discovered = (machines.data?.discovered ?? []).filter(
    (d) => !saved.some((s) => s.ip === d.info.identity.ip),
  );

  const run = async (action: () => Promise<unknown>) => {
    setActionError(null);
    try {
      await action();
      await machines.refresh();
    } catch (e) {
      setActionError(e instanceof Error ? e.message : String(e));
    }
  };

  const scan = () =>
    run(async () => {
      setScanning(true);
      try {
        await client.discover();
      } finally {
        setScanning(false);
      }
    });

  const testConnection = async (ip: string) => {
    setActionError(null);
    setTestResult(null);
    setTestingIp(ip);
    try {
      setTestResult(await client.machineInfo(ip));
    } catch (e) {
      setActionError(e instanceof Error ? e.message : String(e));
    } finally {
      setTestingIp(null);
    }
  };

  return (
    <div className="page">
      {actionError && <ErrorNote>{actionError}</ErrorNote>}

      <Section
        title="Saved machines"
        actions={
          <button onClick={scan} disabled={scanning}>
            {scanning ? "Scanning network…" : "Scan network"}
          </button>
        }
      >
        {saved.length === 0 ? (
          <EmptyState>
            No machines saved yet. Scan the network, or add one by IP below.
          </EmptyState>
        ) : (
          <ul className="machine-list">
            {saved.map((m) => (
              <li
                key={m.ip}
                className={`machine-row ${selectedIp === m.ip ? "selected" : ""}`}
              >
                <button
                  className="machine-main"
                  onClick={() => setSelectedIp(m.ip)}
                  title="Select this machine for sending"
                >
                  <span className="machine-name">
                    {machineLabel({ nickname: m.nickname, ip: m.ip })}
                  </span>
                  {m.manufacturer && <Pill tone="muted">{m.manufacturer}</Pill>}
                  {selectedIp === m.ip && <Pill tone="ok">selected</Pill>}
                </button>
                <div className="machine-actions">
                  <button
                    onClick={() => testConnection(m.ip)}
                    disabled={testingIp !== null}
                  >
                    {testingIp === m.ip ? "Testing…" : "Test"}
                  </button>
                  <button
                    className="danger"
                    onClick={() =>
                      run(async () => {
                        await client.deleteMachine(m.ip);
                        if (selectedIp === m.ip) setSelectedIp(null);
                      })
                    }
                  >
                    Remove
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Section>

      {discovered.length > 0 && (
        <Section title="Discovered on the network">
          <ul className="machine-list">
            {discovered.map((d) => (
              <li key={d.info.identity.ip} className="machine-row">
                <div className="machine-main">
                  <span className="machine-name">
                    {machineLabel({
                      name: d.info.identity.name,
                      ip: d.info.identity.ip,
                    })}
                  </span>
                  <Pill tone="muted">{d.info.identity.manufacturer}</Pill>
                  <span className="dim">{d.info.identity.model}</span>
                </div>
                <div className="machine-actions">
                  <button
                    onClick={() =>
                      run(() =>
                        client.saveMachine({
                          ip: d.info.identity.ip,
                          nickname: d.info.identity.name ?? undefined,
                          manufacturer: d.info.identity.manufacturer,
                        }),
                      )
                    }
                  >
                    Save
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </Section>
      )}

      <Section title="Add machine manually">
        <form
          className="inline-form"
          onSubmit={(e) => {
            e.preventDefault();
            if (!manualIp.trim()) return;
            run(async () => {
              await client.saveMachine({
                ip: manualIp.trim(),
                nickname: manualNickname.trim() || undefined,
              });
              setManualIp("");
              setManualNickname("");
            });
          }}
        >
          <input
            placeholder="IP address, e.g. 192.168.1.120"
            value={manualIp}
            onChange={(e) => setManualIp(e.target.value)}
          />
          <input
            placeholder="Nickname (optional)"
            value={manualNickname}
            onChange={(e) => setManualNickname(e.target.value)}
          />
          <button type="submit" disabled={!manualIp.trim()}>
            Add
          </button>
        </form>
      </Section>

      {testResult && (
        <Section
          title="Connection test"
          actions={<button onClick={() => setTestResult(null)}>Dismiss</button>}
        >
          <dl className="kv">
            <dt>Machine</dt>
            <dd>
              {testResult.identity.name ?? "—"} · {testResult.identity.model}
            </dd>
            <dt>Firmware</dt>
            <dd>{testResult.identity.firmware ?? "—"}</dd>
            <dt>Serial</dt>
            <dd>{testResult.identity.serial ?? "—"}</dd>
            <dt>Embroidery area</dt>
            <dd>
              {testResult.capabilities.embWidthMm ?? "?"} ×{" "}
              {testResult.capabilities.embHeightMm ?? "?"} mm
            </dd>
            <dt>Max design size</dt>
            <dd>
              {testResult.capabilities.maxFileBytes
                ? formatBytes(testResult.capabilities.maxFileBytes)
                : "—"}
            </dd>
            <dt>Formats</dt>
            <dd>{testResult.capabilities.formats.join(", ")}</dd>
          </dl>
        </Section>
      )}
    </div>
  );
}
