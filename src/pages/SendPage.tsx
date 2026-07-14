/**
 * Send page: pick the target machine, pick a design file, check the
 * machine's storage, send, and watch the upload queue with live progress.
 *
 * The target picker writes the same shared selection the Machines page
 * uses, so choosing a machine in either place keeps both in sync.
 */

import { useEffect, useRef, useState } from "react";
import type { JobRecord, MachinesResponse, MachineStatus } from "../api/types";
import { useBridge } from "../hooks/useBridge";
import { usePolling } from "../hooks/usePolling";
import {
  EmptyState,
  ErrorNote,
  Pill,
  ProgressBar,
  Section,
} from "../components/ui";
import { formatBytes, formatTime, machineLabel } from "../lib/format";

export function SendPage() {
  const { client, selectedIp, setSelectedIp } = useBridge();

  const machines = usePolling<MachinesResponse>(
    client ? () => client.machines() : null,
    5000,
  );
  const machineStatus = usePolling<MachineStatus>(
    client && selectedIp ? () => client.machineStatus(selectedIp) : null,
    10000,
  );
  const jobs = usePolling<JobRecord[]>(client ? () => client.jobs() : null, 1000);

  // Saved machines first, then discovered ones not already saved.
  const saved = machines.data?.saved ?? [];
  const targets = [
    ...saved.map((m) => ({
      ip: m.ip,
      label: machineLabel({ nickname: m.nickname, ip: m.ip }),
    })),
    ...(machines.data?.discovered ?? [])
      .filter((d) => !saved.some((s) => s.ip === d.info.identity.ip))
      .map((d) => ({
        ip: d.info.identity.ip,
        label: machineLabel({ name: d.info.identity.name, ip: d.info.identity.ip }),
      })),
  ];

  // The single-machine household shouldn't need a click: default to the
  // first known machine. Never auto-clear — a briefly-offline machine keeps
  // its selection and shows its status error instead.
  const firstIp = targets[0]?.ip ?? null;
  useEffect(() => {
    if (!selectedIp && firstIp) setSelectedIp(firstIp);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedIp, firstIp]);

  // Re-check status the moment the target changes; the 10 s poll cadence is
  // for idling, not for answering a just-made selection.
  useEffect(() => {
    if (selectedIp) void machineStatus.refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedIp]);

  const fileInput = useRef<HTMLInputElement>(null);
  const [file, setFile] = useState<File | null>(null);
  const [sendError, setSendError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);

  if (!client) return null;

  const send = async () => {
    if (!file || !selectedIp) return;
    setSendError(null);
    setSending(true);
    try {
      await client.send(selectedIp, file.name, await file.arrayBuffer());
      setFile(null);
      if (fileInput.current) fileInput.current.value = "";
      await jobs.refresh();
    } catch (e) {
      setSendError(e instanceof Error ? e.message : String(e));
    } finally {
      setSending(false);
    }
  };

  const storage = machineStatus.data?.storage;
  const info = machineStatus.data?.info;

  return (
    <div className="page">
      <Section title="Target machine">
        {targets.length === 0 && !selectedIp ? (
          <EmptyState>
            No machines known yet — scan on the Machines page, or set up a
            dongle.
          </EmptyState>
        ) : (
          <select
            className="machine-select"
            value={selectedIp ?? ""}
            onChange={(e) => setSelectedIp(e.target.value || null)}
          >
            {/* Keep a vanished-but-selected machine choosable rather than
                silently retargeting the send. */}
            {selectedIp && !targets.some((t) => t.ip === selectedIp) && (
              <option value={selectedIp}>{selectedIp} (not seen right now)</option>
            )}
            {targets.map((t) => (
              <option key={t.ip} value={t.ip}>
                {t.label}
              </option>
            ))}
          </select>
        )}
        {!selectedIp ? null : machineStatus.error ? (
          <ErrorNote>
            {selectedIp}: {machineStatus.error}
          </ErrorNote>
        ) : !machineStatus.data ? (
          <EmptyState>Contacting {selectedIp}…</EmptyState>
        ) : (
          <>
            <div className="target-header">
              <strong>
                {info?.identity.name ?? info?.identity.model} · {selectedIp}
              </strong>
              <Pill tone="ok">online</Pill>
            </div>
            {storage && (
              <>
                <div className="storage-line">
                  <span>
                    Memory: {formatBytes(storage.usedBytes)} used of{" "}
                    {formatBytes(storage.totalBytes)}
                  </span>
                  <span className="dim">
                    {formatBytes(storage.freeBytes)} free
                  </span>
                </div>
                <ProgressBar value={storage.usedBytes} max={storage.totalBytes} />
                {storage.files.length > 0 && (
                  <p className="dim file-list">
                    On machine: {storage.files.join(", ")}
                  </p>
                )}
              </>
            )}
          </>
        )}
      </Section>

      <Section title="Send a design">
        {sendError && <ErrorNote>{sendError}</ErrorNote>}
        <div className="send-controls">
          <input
            ref={fileInput}
            type="file"
            accept=".pes,.phc,.dst,.phx,.jef,.exp,.vp3,.xxx"
            onChange={(e) => setFile(e.target.files?.[0] ?? null)}
          />
          <button
            className="primary"
            onClick={send}
            disabled={!file || !selectedIp || sending}
          >
            {sending ? "Queuing…" : "Send to machine"}
          </button>
        </div>
        {file && (
          <p className="dim">
            {file.name} · {formatBytes(file.size)}
          </p>
        )}
      </Section>

      <Section title="Upload queue">
        {!jobs.data || jobs.data.length === 0 ? (
          <EmptyState>Nothing sent yet.</EmptyState>
        ) : (
          <ul className="job-list">
            {jobs.data.map((job) => (
              <li key={job.id} className="job-row">
                <div className="job-line">
                  <span className="job-name">{job.filename}</span>
                  <span className="dim">→ {job.ip}</span>
                  <JobStatePill job={job} />
                  <span className="dim job-time">
                    {formatTime(job.createdAtMs)}
                  </span>
                </div>
                {job.state === "uploading" && (
                  <ProgressBar value={job.sentBytes} max={job.totalBytes} />
                )}
                {job.state === "done" && job.storedAs && (
                  <p className="dim">Stored on machine as {job.storedAs}</p>
                )}
                {job.state === "failed" && job.error && (
                  <p className="job-error">{job.error}</p>
                )}
              </li>
            ))}
          </ul>
        )}
      </Section>
    </div>
  );
}

function JobStatePill({ job }: { job: JobRecord }) {
  switch (job.state) {
    case "queued":
      return <Pill tone="muted">queued</Pill>;
    case "uploading":
      return (
        <Pill tone="warn">
          uploading · {formatBytes(job.sentBytes)} / {formatBytes(job.totalBytes)}
        </Pill>
      );
    case "done":
      return <Pill tone="ok">done</Pill>;
    case "failed":
      return <Pill tone="err">failed</Pill>;
  }
}
