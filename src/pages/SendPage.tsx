/**
 * Send page: pick a design file, check the selected machine's storage, send,
 * and watch the upload queue with live progress.
 */

import { useRef, useState } from "react";
import type { JobRecord, MachineStatus } from "../api/types";
import { useBridge } from "../hooks/useBridge";
import { usePolling } from "../hooks/usePolling";
import {
  EmptyState,
  ErrorNote,
  Pill,
  ProgressBar,
  Section,
} from "../components/ui";
import { formatBytes, formatTime } from "../lib/format";

export function SendPage() {
  const { client, selectedIp } = useBridge();

  const machineStatus = usePolling<MachineStatus>(
    client && selectedIp ? () => client.machineStatus(selectedIp) : null,
    10000,
  );
  const jobs = usePolling<JobRecord[]>(client ? () => client.jobs() : null, 1000);

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
        {!selectedIp ? (
          <EmptyState>
            No machine selected. Choose one on the Machines page.
          </EmptyState>
        ) : machineStatus.error ? (
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
