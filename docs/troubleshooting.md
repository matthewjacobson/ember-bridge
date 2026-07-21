# Troubleshooting

Most problems come down to the network or a permission. Start here, and check
the **Logs** page — it shows what the bridge is doing in real time.

## "machine unreachable" / Ember Bridge can't reach the machine

The bridge found no path to the machine. Work through:

- **Same network?** Your computer and the machine (or dongle) must be on the
  same WiFi network. Watch out for separate guest networks and 5 GHz-only bands
  — dongles only join **2.4 GHz**.
- **macOS Local Network permission.** Open **System Settings → Privacy &
  Security → Local Network** and make sure **Ember Bridge** is switched on. This
  is the most common cause of "machine unreachable" when the machine otherwise
  seems fine (you can even ping it, but the app can't reach it).
- **VPN off?** A VPN can route traffic away from your home network.
- **Router isolation.** Some routers isolate devices on guest networks so they
  can't see each other — use your main network.
- **Dongle joined WiFi?** If it's a dongle, confirm setup finished and it joined
  your network.

## The footer says "API offline" (or Ember says the bridge isn't responding)

The local service didn't start. Usually this means **another copy of Ember
Bridge is already running**. Quit any extra copies (or restart the app). Ember
Bridge uses port **17831** on your computer.

## Ember won't connect even though the bridge is running

- Approve the pairing prompt in Ember Bridge, or paste the token — see
  [Connecting Ember](connecting-ember.md).
- If you use Ember from a non-official address, add it under **Settings →
  Allowed web origins**.
- Make sure you copied the **entire** token.

## "machine requires pairing" (dongle)

The dongle's pairing window has closed. **Unplug and replug the dongle**
(power-cycle it) and try again within **5 minutes**.

## Wrong WiFi password during dongle setup

Re-enter the password and try **Connect** again. Also remember only **2.4 GHz**
networks show in the list — if yours is missing, type it in by hand.

## "design is … bytes but the machine accepts at most …" / "not enough free memory"

The design is too large for the machine, or its memory is full. Simplify or
reduce the design, or free up space by removing designs already stored on the
machine.

## "unsupported design format"

The machine doesn't accept that file type. Re-export the design from your design
software in a format the machine supports (see the format tables in
[Adding machines](adding-machines.md) and [Sending a design](sending-designs.md)).

## Still stuck?

Open the **Logs** page and reproduce the problem — the messages there (and any
error text shown in the app) describe exactly what happened and are the best
starting point for getting help.
