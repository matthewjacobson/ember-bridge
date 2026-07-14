/**
 * Dongle setup page: the out-of-box flow for an EmberConnect dongle plugged
 * into *this* computer via USB.
 *
 * plug in → pick a WiFi network (scanned by the dongle's own radio) → enter
 * password + name the machine → the dongle joins the network live (nothing
 * saved on failure, so a wrong password is an inline retry) → unplug and
 * move it to the embroidery machine. Provisioning also pre-pairs this
 * Bridge, so the machine is usable the moment it appears on the network.
 */

import { useEffect, useRef, useState } from "react";
import {
  asDongleError,
  dongleInfo,
  listDongles,
  onUpdateProgress,
  provisionDongle,
  scanNetworks,
  updateDongleFirmware,
  type DongleNetwork,
  type DongleSetupInfo,
  type ProvisionOutcome,
  type UpdateProgress,
} from "../api/dongle";
import { useBridge } from "../hooks/useBridge";
import { usePolling } from "../hooks/usePolling";
import { EmptyState, ErrorNote, Pill, ProgressBar, Section } from "../components/ui";

function signalBars(rssi: number): string {
  return rssi > -55 ? "▮▮▮" : rssi > -70 ? "▮▮▯" : "▮▯▯";
}

export function SetupPage() {
  const { client } = useBridge();
  const dongles = usePolling(listDongles, 2000);
  const dongle = dongles.data?.[0] ?? null;
  const port = dongle?.port ?? null;

  const [info, setInfo] = useState<DongleSetupInfo | null>(null);
  const [networks, setNetworks] = useState<DongleNetwork[] | null>(null);
  const [scanning, setScanning] = useState(false);

  const [ssid, setSsid] = useState("");
  const [password, setPassword] = useState("");
  const [machineName, setMachineName] = useState("");
  const passwordRef = useRef<HTMLInputElement>(null);

  const [provisioning, setProvisioning] = useState(false);
  const [done, setDone] = useState<ProvisionOutcome | null>(null);
  const [savedToMachines, setSavedToMachines] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // One serial conversation at a time: the port is exclusive, so info
  // fetches and scans must never race a provision or update.
  const busy = provisioning || scanning;

  const rescan = async (targetPort: string) => {
    setScanning(true);
    setError(null);
    try {
      setNetworks(await scanNetworks(targetPort));
    } catch (e) {
      setError(asDongleError(e).message);
    } finally {
      setScanning(false);
    }
  };

  // A dongle appeared (or was swapped): read its state, scan for networks.
  useEffect(() => {
    if (!port) {
      setInfo(null);
      setNetworks(null);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const i = await dongleInfo(port);
        if (cancelled) return;
        setInfo(i);
        if (i.deviceName) setMachineName(i.deviceName);
        if (i.wifi.configuredSsid) setSsid(i.wifi.configuredSsid);
        await rescan(port);
      } catch (e) {
        if (!cancelled) setError(asDongleError(e).message);
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [port]);

  const provision = async () => {
    if (!port || !ssid.trim()) return;
    setProvisioning(true);
    setError(null);
    try {
      const outcome = await provisionDongle(
        port,
        ssid.trim(),
        password,
        machineName.trim(),
      );
      // Finish the job: put the dongle on the Machines page under the name
      // just chosen, replacing any stale entry at that address. Best-effort —
      // the dongle is provisioned either way, and discovery finds it anyway.
      let saved = false;
      if (client && outcome.ip) {
        try {
          await client.saveMachine({
            ip: outcome.ip,
            nickname: machineName.trim() || undefined,
            manufacturer: "emberconnect",
          });
          saved = true;
        } catch {
          // Non-fatal; the done screen just omits the "saved" line.
        }
      }
      setSavedToMachines(saved);
      setDone(outcome);
    } catch (e) {
      const err = asDongleError(e);
      if (err.code === "wrong_password") {
        setError(`"${ssid.trim()}" rejected the password — try again.`);
        passwordRef.current?.focus();
      } else {
        setError(err.message);
      }
    } finally {
      setProvisioning(false);
    }
  };

  if (done) {
    return (
      <div className="page">
        <Section title="Dongle ready">
          <p>
            <Pill tone="ok">connected</Pill> The dongle joined{" "}
            <strong>{done.ssid}</strong>
            {done.ip && ` (${done.ip})`}
            {done.paired && " and is already paired with this Bridge"}.
            {savedToMachines &&
              " It has been added to your Machines page" +
                (machineName.trim() ? ` as "${machineName.trim()}"` : "") +
                "."}
          </p>
          <p>
            <strong>Unplug it from this computer and plug it into your
            embroidery machine.</strong>{" "}
            It will reconnect to your WiFi on its own
            {savedToMachines
              ? " and be ready to sew."
              : " and show up on the Machines page."}
          </p>
          <button
            onClick={() => {
              setDone(null);
              setPassword("");
            }}
          >
            Set up another dongle
          </button>
        </Section>
      </div>
    );
  }

  return (
    <div className="page">
      {error && <ErrorNote>{error}</ErrorNote>}

      {!dongle ? (
        <Section title="Set up a dongle">
          <EmptyState>
            Plug an EmberConnect dongle into a USB port on this computer.
            It will be detected automatically.
          </EmptyState>
        </Section>
      ) : (
        <>
          <Section title="Dongle">
            <dl className="kv">
              <dt>Serial</dt>
              <dd>{info?.serial ?? dongle.serial ?? "—"}</dd>
              <dt>Firmware</dt>
              <dd>{info?.version ?? "—"}</dd>
              <dt>Status</dt>
              <dd>
                {info === null ? (
                  "—"
                ) : info.wifi.connected ? (
                  <>
                    <Pill tone="ok">on WiFi</Pill> {info.wifi.configuredSsid} (
                    {info.wifi.ip})
                  </>
                ) : info.provisioned ? (
                  <Pill tone="warn">configured, not connected</Pill>
                ) : (
                  <Pill tone="muted">new — needs WiFi</Pill>
                )}
              </dd>
            </dl>
          </Section>

          <Section
            title="Choose your WiFi network"
            actions={
              <button onClick={() => port && rescan(port)} disabled={busy}>
                {scanning ? "Scanning…" : "Rescan"}
              </button>
            }
          >
            {networks === null ? (
              <EmptyState>Scanning for networks…</EmptyState>
            ) : networks.length === 0 ? (
              <EmptyState>
                No networks found — rescan, or type the network name below.
              </EmptyState>
            ) : (
              <ul className="machine-list">
                {networks.map((n) => (
                  <li
                    key={n.ssid}
                    className={`machine-row ${ssid === n.ssid ? "selected" : ""}`}
                  >
                    <button
                      className="machine-main"
                      disabled={busy}
                      onClick={() => {
                        setSsid(n.ssid);
                        passwordRef.current?.focus();
                      }}
                    >
                      <span className="machine-name">
                        {n.secure ? "🔒 " : ""}
                        {n.ssid}
                      </span>
                      <span className="dim">{signalBars(n.rssi)}</span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <p className="dim">
              The list comes from the dongle's own radio, so it only shows
              networks it can join. It can't see 5 GHz-only networks — use
              your router's 2.4 GHz band.
            </p>
          </Section>

          <Section title="Connect">
            <form
              className="inline-form"
              onSubmit={(e) => {
                e.preventDefault();
                void provision();
              }}
            >
              <input
                placeholder="Network name (or pick above)"
                value={ssid}
                disabled={busy}
                onChange={(e) => setSsid(e.target.value)}
              />
              <input
                ref={passwordRef}
                type="password"
                placeholder="WiFi password"
                value={password}
                disabled={busy}
                onChange={(e) => setPassword(e.target.value)}
              />
              <input
                placeholder="Machine name, e.g. Sewing room Brother"
                value={machineName}
                disabled={busy}
                onChange={(e) => setMachineName(e.target.value)}
              />
              <button
                type="submit"
                className="primary"
                disabled={busy || !ssid.trim()}
              >
                {provisioning ? "Connecting dongle…" : "Connect"}
              </button>
            </form>
            {provisioning && (
              <p className="dim">
                The dongle is trying to join "{ssid.trim()}". Nothing is saved
                unless the join succeeds — this takes up to 30 seconds.
              </p>
            )}
          </Section>

          {info && port && (
            <FirmwareUpdate port={port} version={info.version} busy={busy} />
          )}
        </>
      )}
    </div>
  );
}

/**
 * Manual firmware push from a signed .bin. A stopgap for support/dev use
 * until Bridge checks a release manifest and offers updates on its own
 * (roadmap) — hence a path field rather than a native file picker.
 */
function FirmwareUpdate({
  port,
  version,
  busy,
}: {
  port: string;
  version: string;
  busy: boolean;
}) {
  const [imagePath, setImagePath] = useState("");
  const [updating, setUpdating] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [result, setResult] = useState<string | null>(null);

  const update = async () => {
    setUpdating(true);
    setResult(null);
    setProgress(null);
    const unlisten = await onUpdateProgress(setProgress);
    try {
      await updateDongleFirmware(port, imagePath.trim());
      setResult(
        "Update verified — the dongle is rebooting and will reappear in a few seconds.",
      );
      setImagePath("");
    } catch (e) {
      setResult(`Update failed: ${asDongleError(e).message}`);
    } finally {
      unlisten();
      setUpdating(false);
      setProgress(null);
    }
  };

  return (
    <Section title="Firmware update (advanced)">
      <p className="dim">
        Running version {version}. Point at a signed EmberConnect image
        (ember-connect.bin) to update over USB — the dongle rejects anything
        not signed with the EmberConnect key.
      </p>
      <form
        className="inline-form"
        onSubmit={(e) => {
          e.preventDefault();
          void update();
        }}
      >
        <input
          placeholder="/path/to/ember-connect.bin"
          value={imagePath}
          disabled={busy || updating}
          onChange={(e) => setImagePath(e.target.value)}
        />
        <button type="submit" disabled={busy || updating || !imagePath.trim()}>
          {updating ? "Updating…" : "Update firmware"}
        </button>
      </form>
      {progress && <ProgressBar value={progress.written} max={progress.total} />}
      {result && <p>{result}</p>}
    </Section>
  );
}
