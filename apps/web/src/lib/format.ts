// Small formatting helpers shared across the UI.

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) {
    return "—";
  }
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = "B";
  for (const u of units) {
    value /= 1024;
    unit = u;
    if (value < 1024) {
      break;
    }
  }
  return `${value >= 100 ? Math.round(value) : value.toFixed(1)} ${unit}`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) {
    return `${Math.round(ms)} ms`;
  }
  const s = ms / 1000;
  if (s < 60) {
    return `${s.toFixed(1)} s`;
  }
  const m = Math.floor(s / 60);
  return `${m}m ${Math.round(s - m * 60)}s`;
}

export function humanPhase(phase: string): string {
  switch (phase) {
    case "awaiting-metadata":
      return "Waiting for the metadata frame…";
    case "collecting":
      return "Collecting packets";
    case "decoding":
      return "Reconstructing";
    case "verifying":
      return "Verifying integrity";
    case "verified":
      return "Verified";
    case "failed":
      return "Failed";
    default:
      return phase;
  }
}
