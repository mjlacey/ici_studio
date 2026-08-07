// Shared blob-download helper -- greenfield, no existing download pattern
// anywhere else in this codebase.

export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

export function downloadText(text: string, filename: string, mimeType = "text/plain;charset=utf-8"): void {
  downloadBlob(new Blob([text], { type: mimeType }), filename);
}

export function downloadJson(data: unknown, filename: string): void {
  downloadText(JSON.stringify(data, null, 2), filename, "application/json;charset=utf-8");
}
