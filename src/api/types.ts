/**
 * TypeScript mirrors of the localhost API's JSON shapes.
 * Keep in sync with the Rust models in `src-tauri/src/machine/models.rs`
 * and `src-tauri/src/server/`.
 */

export interface MachineIdentity {
  manufacturer: string;
  model: string;
  name: string | null;
  firmware: string | null;
  serial: string | null;
  ip: string;
}

export interface MachineCapabilities {
  embWidthMm: number | null;
  embHeightMm: number | null;
  needles: number | null;
  maxFileBytes: number | null;
  formats: string[];
}

export interface MachineInfo {
  identity: MachineIdentity;
  capabilities: MachineCapabilities;
}

export interface StorageStatus {
  totalBytes: number;
  freeBytes: number;
  usedBytes: number;
  files: string[];
}

export interface MachineStatus {
  info: MachineInfo;
  storage: StorageStatus;
}

export interface SavedMachine {
  ip: string;
  nickname?: string;
  manufacturer?: string;
}

export interface DiscoveredMachine {
  info: MachineInfo;
}

export interface MachinesResponse {
  saved: SavedMachine[];
  discovered: DiscoveredMachine[];
  discoveryCompletedAtMs: number | null;
  discoveryRunning: boolean;
}

export type JobState = "queued" | "uploading" | "done" | "failed";

export interface JobRecord {
  id: string;
  filename: string;
  ip: string;
  state: JobState;
  sentBytes: number;
  totalBytes: number;
  storedAs: string | null;
  errorCode: string | null;
  error: string | null;
  createdAtMs: number;
  finishedAtMs: number | null;
}

export type LogLevel = "info" | "warn" | "error";

export interface LogEntry {
  seq: number;
  timestampMs: number;
  level: LogLevel;
  message: string;
}

export interface LogsResponse {
  entries: LogEntry[];
  lastSeq: number;
}

export interface BridgeStatus {
  app: string;
  version: string;
  apiVersion: number;
  uptimeSeconds: number;
  server: { running: boolean; port: number; error: string | null };
  pendingUploads: number;
  savedMachines: number;
  discoveryRunning: boolean;
}

export interface Settings {
  apiToken: string;
  allowedOrigins: string[];
  port: number;
}

/** Structured error body: `{"error": {"code", "message"}}`. */
export interface ApiErrorBody {
  error: { code: string; message: string };
}
