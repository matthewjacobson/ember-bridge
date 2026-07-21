# Setting up a dongle

An **EmberConnect** dongle adds WiFi to an embroidery machine that doesn't have
it. You set the dongle up once by plugging it into your computer over USB, tell
it which WiFi network to join, then move it to your machine.

## Before you start

Have your **2.4 GHz** WiFi network name and password handy. The dongle's radio
can only join 2.4 GHz networks — it can't see or use 5 GHz-only networks. If
your router uses one name for both bands, that's fine.

## Step by step

1. **Plug the dongle into a USB port on this computer.** Ember Bridge detects it
   automatically and an **Ember Connect Set Up** entry appears in the sidebar.
   Open it.

2. **Check the dongle.** The page shows the dongle's **Serial**, **Firmware**,
   and **Status** — one of *new — needs WiFi*, *configured, not connected*, or
   *on WiFi*.

3. **Choose your WiFi network.** Under **Choose your WiFi network**, pick your
   network from the list. A lock (🔒) marks secured networks and the bars show
   signal strength. Don't see it? Click **Rescan**, or type the name in by hand.
   Remember: only 2.4 GHz networks appear here.

4. **Connect.** Enter the **WiFi password** and a **Machine name** (for example
   "Sewing room Brother"), then click **Connect**. The dongle actually tries to
   join the network before anything is saved, so this can take up to ~30
   seconds. If the password is wrong you'll see *"…rejected the password — try
   again."* — just re-enter it.

5. **Done.** When it succeeds you'll see **Dongle ready**: the dongle joined
   your network (with its assigned address), was automatically paired with this
   Bridge, and was added to your **Machines** page.

Now **unplug the dongle from your computer and plug it into your embroidery
machine.** It reconnects to your WiFi on its own and is ready to sew — no
further setup needed. To set up another dongle, click **Set up another dongle**.

## If the machine asks to pair later

If a dongle-equipped machine ever reports that pairing is required, its pairing
window has closed. **Unplug and replug the dongle** (power-cycle it) and try
again within **5 minutes**.

## Firmware update (advanced)

While a dongle is connected over USB, a **Firmware update (advanced)** section
lets you update it:

1. Point the path field at a signed EmberConnect image file
   (`ember-connect.bin`).
2. Click **Update firmware** and watch the progress bar.

The dongle only accepts images signed with the official EmberConnect key, so it
will reject anything else. When it finishes, the dongle verifies the update,
reboots, and reappears after a few seconds. You normally won't need this unless
directed to update.
