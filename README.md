# Ember Bridge

Desktop companion for the **Ember** web-based embroidery editor. Bridges the
browser to WiFi-capable embroidery machines:

```
Browser (Ember) ──HTTP──▶ 127.0.0.1:17831 (Ember Bridge) ──HTTPS──▶ machine
```

Built with **Tauri v2** (Rust backend, React + TypeScript + Vite frontend).
Currently supports **Brother** machines (Innov-is / WLAN line) via the
reverse-engineered "pedxml" protocol spoken by Brother's *Design Database
Transfer* application.

## Why a desktop bridge?

Browsers cannot talk to the machines directly: the machines use HTTPS with a
self-signed certificate and a legacy TLS 1.2 static-RSA cipher suite, which no
browser will connect to from a web page. Ember Bridge terminates that quirky
TLS on the desktop and exposes a small, token-protected REST API on loopback.

## Development

```bash
npm install
npm run tauri dev            # runs vite + cargo, opens the app
cd src-tauri && cargo test   # protocol unit tests + API integration tests
```

Requires Rust (stable) and Node 20+. Note: Vite is pinned to v6 because v7
requires Node ≥ 20.19.

## Localhost API (what Ember calls)

All endpoints are bound to `127.0.0.1:17831` only. Every request except
`GET /api/health` and the browser side of pairing requires the API token,
sent as `Authorization: Bearer <token>` or `X-Ember-Token: <token>`. Browser
callers must also have their origin added to the allowlist on the app's
**Settings** page (CORS); localhost origins are always allowed.

A site obtains the token by **pairing**: `POST /api/pair` (the desktop app
surfaces an Approve/Deny prompt naming the requesting origin), then poll
`GET /api/pair/{id}` until it answers `{state: "approved", token}` — the
token is released exactly once, only to the origin that asked, and requests
expire after two minutes. The token is also shown on the Settings page for
manual setup; `examples/ember-demo.html` demonstrates both paths.

| Method | Path                | Purpose |
|--------|---------------------|---------|
| GET    | `/api/health`       | Liveness + version (no token) |
| GET    | `/api/status`       | Bridge status; with `?ip=` → live machine status (storage + files) |
| GET    | `/api/info?ip=`     | Identify a machine (model, firmware, capabilities) |
| GET    | `/api/machines`     | Saved machines + last discovery results |
| POST   | `/api/machines`     | Save a machine `{ip, nickname?}` |
| DELETE | `/api/machines/{ip}`| Forget a saved machine |
| POST   | `/api/discover`     | Sweep the local network (blocks a few seconds) |
| POST   | `/api/pair`         | Ask to pair `{appName?}` (no token; origin-gated). Returns `202 {request}` |
| GET    | `/api/pair/{id}`    | Poll a pairing request (no token); `approved` carries the token, once |
| GET    | `/api/pairing`      | Pending pairing request — consumed by the app's own UI banner |
| POST   | `/api/pairing/respond` | Approve/deny `{id, approve}` — app UI only |
| POST   | `/api/send?ip=&filename=` | Enqueue an upload; body = raw design bytes. Returns `202 {job}` |
| GET    | `/api/jobs`, `/api/jobs/{id}` | Upload queue / progress polling |
| GET    | `/api/logs?afterSeq=` | Incremental app log |
| GET/PUT| `/api/settings`     | Token, allowed origins |

Errors are structured: `{"error": {"code": "insufficient_storage", "message": "…"}}`.
Useful codes: `unauthorized`, `invalid_ip`, `ip_not_local`, `not_a_machine`,
`machine_unreachable`, `machine_timeout`, `machine_rejected`, `file_too_large`,
`insufficient_storage`, `unsupported_format`, `upload_failed`.

Typical Ember flow:

```js
const headers = { Authorization: `Bearer ${token}` };
await fetch("http://127.0.0.1:17831/api/health");                 // bridge installed?
const { job } = await (await fetch(
  `http://127.0.0.1:17831/api/send?ip=${ip}&filename=rose.pes`,
  { method: "POST", headers, body: pesBytes })).json();           // send
// then poll /api/jobs/{job.id} until state is "done" or "failed"
```

## Architecture

```
src-tauri/src/
  machine/          Manufacturer-NEUTRAL layer — the only machine API the
    mod.rs          rest of the app sees. Traits: EmbroideryMachine (info /
    models.rs       storage / upload), MachineBackend (probe / connect /
    error.rs        discover), plus neutral models and errors, and the
    net.rs          local-network IP policy.
  emberconnect/     EmberConnect dongle backend (our own WiFi "memory
    client.rs       stick" hardware; see the EmberConnect repo). Plain
    models.rs       HTTP/JSON on port 80; discovery via mDNS browse
    discovery.rs    (_ember-connect._tcp) instead of a subnet sweep.
    tokens.rs       Pairing tokens by dongle serial (firmware 0.4.0+
                    requires them); pairing happens transparently on 401
                    while the dongle's pairing window is open, else the
                    user is told to replug it (pairing_required).
  dongle_setup/     Desktop (USB) setup for EmberConnect dongles: the
    mod.rs          "plug it into your computer first" out-of-box flow.
    link.rs         CDC-ACM serial transport (line-delimited JSON; protocol
                    defined in the EmberConnect repo, usb_setup.h). Scans
                    WiFi with the dongle's radio, live-trials credentials
                    (commit-on-success → wrong password is an inline retry),
                    names the machine, pre-pairs this Bridge (writes into
                    the shared TokenStore), pushes signed firmware. Exposed
                    as Tauri commands, NOT on the localhost REST API —
                    browser origins have no business provisioning hardware.
  brother/          Brother backend (pedxml protocol).
    protocol.rs     Pure wire format: byte-exact request builders + tolerant
                    XML response parsing. No I/O; heavily unit-tested.
    client.rs       HTTPS client with all transport quirks (see below).
    models.rs       Serde types for /info JSON and sewing.cgi XML.
    discovery.rs    Active /24 sweep (TCP dial → protocol probe).
  server/           Localhost REST API (axum).
    routes.rs       Handlers — manufacturer-agnostic by construction.
    auth.rs         Bearer-token middleware + hand-rolled CORS (dynamic
                    origin allowlist, Private Network Access preflight).
    jobs.rs         Upload queue: single worker, serialized uploads,
                    progress via polling.
    error.rs        MachineError → structured JSON error mapping.
    state.rs        Shared AppState.
  config.rs         Persisted config (token, saved machines, origins);
                    atomic writes, 0600 permissions.
  logging.rs        In-memory ring-buffer log behind /api/logs.
  lib.rs / main.rs  Tauri wiring. One Tauri command (`local_api_info`)
                    hands the UI the port + token; the React UI then uses
                    the same REST API as Ember.

src/
  api/              Typed client for the bridge API (mirrors Rust models),
                    plus dongle.ts (dongle-setup Tauri command wrappers).
  hooks/            BridgeProvider (client + selected machine), usePolling.
  pages/            Machines, Set up dongle, Send, Logs, Settings.
  components/       Small presentational pieces.
```

### Adding a new manufacturer (Janome, Bernina, Baby Lock, …)

1. Create `src-tauri/src/<manufacturer>/` implementing `EmbroideryMachine`
   and `MachineBackend` (use `brother/` as the template: keep the wire format
   in a pure, testable `protocol.rs`).
2. Register it in `BackendRegistry::with_default_backends()`
   (`machine/mod.rs`).

That's it — discovery, the REST API, the job queue, and the UI are all
manufacturer-agnostic and pick the new backend up automatically. The
registry probes each backend to answer "what is at this IP?".

## Brother protocol notes (hard-won details)

Reference: packet captures of *Design Database Transfer* (see the
`brother-embroidery-connect` PoC repo and its `PROTOCOL.md`).

* **TLS**: self-signed cert (CN like `56;2;1;1.73.local`), TLS 1.2 only,
  cipher `TLS_RSA_WITH_AES_256_GCM_SHA384` — **static RSA key exchange**.
  rustls refuses to implement static-RSA suites, so the client must use
  reqwest's **native-tls** backend (SecureTransport / SChannel / OpenSSL).
  Do not "upgrade" this to rustls; the handshake will fail on real hardware.
* `GET /info` → JSON identity; a device is a supported machine iff
  `apis.pedxml` is present. `features.embwidth/embheight` are in 0.1 mm,
  `features.postsize` is the max upload size in bytes.
* `POST /sewing/sewing.cgi` (form-encoded, `req_appstate=2`) → XML status:
  memory total/free + file list. Note the firmware misspells the XML root
  (`<respose_info>`); parsing is tag-extraction, deliberately tolerant.
* Upload = same endpoint, multipart, `req_appstate=3`, two parts
  (`req_parameter`, `myfile`). The multipart body is built **by hand**,
  byte-for-byte as captured (nonstandard `Content-Disposition:` formatting
  without spaces) because embedded CGI parsers are strict. Success is HTTP
  **204**; the machine renames the file itself (e.g. `32770.PES`) — we diff
  the file list before/after to report the assigned name.
* The embedded server (`debut/1.20`) is slow and single-threaded: generous
  timeouts, retries on reads, at most one retry on upload (to avoid
  duplicate designs), no connection pooling, explicit `Content-Length`
  (no chunked encoding).
* Discovery: the machines do not announce via mDNS; we sweep each private
  /24 with a short TCP dial to :443 followed by a protocol probe.

## Security model

* The API binds to `127.0.0.1` **only** — never reachable off-machine.
* A random 128-bit token (generated on first launch, stored 0600) is
  required on every request; localhost is not treated as trusted because any
  web page can *send* requests to loopback.
* CORS: browsers only get responses for origins on the user-managed
  allowlist (Settings page). Chrome's Private Network Access preflight is
  answered. `/api/health` is origin-agnostic so Ember can detect the bridge.
* Pairing releases the token only after a human clicks Approve in the
  desktop window, only to the origin named in the prompt, and only once;
  initiation is refused server-side for non-allowlisted origins.
* DNS-rebinding guard: requests whose `Host` header is not loopback are
  rejected outright (a rebound page bypasses CORS but cannot fake `Host`).
* Target IPs must be private/link-local IPv4 — the bridge refuses to proxy
  to loopback or the public internet.
* Design size is validated against the machine's advertised `postsize` and
  live free memory before any bytes are sent; every network call has a
  timeout.

## Status / roadmap

Implemented: discovery, identification, storage query, upload queue with
progress, machine nicknames, logs, settings, one-click browser pairing
(Approve/Deny prompt in the app), EmberConnect dongle backend (mDNS
discovery + HTTP upload — covers machines with no network hardware via our
own USB dongle).

Future: system-tray mode, autostart, per-machine upload history, additional
manufacturer backends.
