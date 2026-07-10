/** Small display formatters shared across pages. */

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes)) return "–";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

export function formatTime(ms: number): string {
  return new Date(ms).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/** "BETTY (192.168.1.120)" or the nickname when the user set one. */
export function machineLabel(opts: {
  nickname?: string | null;
  name?: string | null;
  ip: string;
}): string {
  const title = opts.nickname || opts.name;
  return title ? `${title} (${opts.ip})` : opts.ip;
}
