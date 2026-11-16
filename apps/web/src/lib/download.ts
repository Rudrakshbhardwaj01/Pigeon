// Local download helper — creates an object URL, triggers the download and
// revokes the URL afterwards (no network involved).

export function downloadBlob(data: BlobPart | Uint8Array, filename: string, mime = "application/octet-stream"): void {
  const blob = new Blob([data as BlobPart], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 5_000);
}
