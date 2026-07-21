# Sending a design

The **Send** page is where you push a design file to a machine.

## 1. Pick a target machine

Choose a machine from the **Target machine** dropdown. Saved machines are listed
first, then any that were discovered but not yet saved. If you only have one
machine, it's selected for you. The choice is shared with the Machines page.

When the machine is reachable you'll see it marked **online**, along with:

- **Memory** — how much of the machine's storage is used, of its total, and how
  much is free, with a bar showing the split.
- **On machine** — the design files currently stored on it.

## 2. Choose a file and send

Click the file picker and choose a design, then click **Send to machine**. The
button reads **Queuing…** briefly while it starts. Below the picker you'll see
the file name and size you selected.

### Which file formats work?

The file picker shows a broad list of embroidery formats, but **each machine
only accepts its own**:

| Machine | Accepted formats |
|---|---|
| Brother | pes, phc, dst, phx |
| EmberConnect dongle | pes, pec, dst, exp, jef, vp3, hus, vip, xxx |

If you send a format a machine doesn't accept, the upload fails with a message
telling you which formats it does accept.

### Size limits

A design can be up to **32 MB**. Individual machines also have their own, usually
much smaller, per-file limit (often around 3 MB) and a finite amount of memory —
if a design is too big or there isn't room, you'll get a clear message saying so.

## 3. Watch the upload queue

Every send appears in the **Upload queue** with its status:

| Status | Meaning |
|---|---|
| queued | Waiting its turn |
| uploading | In progress, with a progress bar |
| done | Delivered — shows **Stored on machine as …** if the machine renamed it |
| failed | Didn't go through — shows the reason |

Uploads run **one at a time**. While any are pending, the **Send** item in the
sidebar shows a badge with the count. Machines commonly rename an uploaded file
to their own numbering scheme — that's normal, and the stored name is shown on
the finished job.
