/**
 * Typed wrappers for the dongle-setup Tauri commands.
 *
 * Unlike everything in `client.ts`, these deliberately bypass the localhost
 * REST API: USB provisioning handles WiFi passwords and local hardware,
 * which paired browser origins must not be able to reach. The wire protocol
 * is the dongle firmware's CDC channel (EmberConnect repo,
 * `firmware/main/usb_setup.h`).
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface DongleSummary {
  port: string;
  serial: string | null;
}

export interface DongleSetupInfo {
  deviceName: string;
  version: string;
  serial: string;
  provisioned: boolean;
  wifi: {
    setupMode: boolean;
    connected: boolean;
    ip: string;
    configuredSsid: string;
    lastError: string;
  };
  update: {
    slot: string;
    pendingVerify: boolean;
    maxImageSize: number;
  };
}

export interface DongleNetwork {
  ssid: string;
  rssi: number;
  secure: boolean;
}

export interface ProvisionOutcome {
  ssid: string;
  ip: string;
  serial: string | null;
  paired: boolean;
}

export interface UpdateProgress {
  written: number;
  total: number;
}

/** Errors from these commands arrive as `{code, message}` objects. */
export interface DongleError {
  code: string;
  message: string;
}

export function asDongleError(e: unknown): DongleError {
  if (e && typeof e === "object" && "code" in e && "message" in e) {
    return e as DongleError;
  }
  return { code: "unknown", message: e instanceof Error ? e.message : String(e) };
}

export function listDongles(): Promise<DongleSummary[]> {
  return invoke("dongle_list");
}

export function dongleInfo(port: string): Promise<DongleSetupInfo> {
  return invoke("dongle_info", { port });
}

export async function scanNetworks(port: string): Promise<DongleNetwork[]> {
  const result = await invoke<{ networks: DongleNetwork[] }>("dongle_scan", {
    port,
  });
  return result.networks;
}

/** Live-trials the credentials; resolves only once the dongle is on the
 *  network (or rejects with `wrong_password` / `network_not_found` /
 *  `join_timeout`). Takes up to ~40 s. */
export function provisionDongle(
  port: string,
  ssid: string,
  password: string,
  name: string,
): Promise<ProvisionOutcome> {
  return invoke("dongle_provision", { port, ssid, password, name });
}

/** Streams a signed image; the dongle verifies and reboots on success. */
export function updateDongleFirmware(
  port: string,
  imagePath: string,
): Promise<unknown> {
  return invoke("dongle_update_firmware", { port, imagePath });
}

export function onUpdateProgress(
  handler: (progress: UpdateProgress) => void,
): Promise<UnlistenFn> {
  return listen<UpdateProgress>("dongle-update-progress", (event) =>
    handler(event.payload),
  );
}
