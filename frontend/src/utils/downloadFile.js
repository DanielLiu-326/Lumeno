// Download a URL through fetch -> blob -> object URL with the download
// attribute. On iOS a plain <a href> navigation hands the file to Safari's
// inline viewer even when the response carries an attachment header; a blob
// URL with the download attribute triggers the system download sheet instead.
export async function downloadFileByUrl(url, fallbackName = "download") {
  const res = await fetch(url, { credentials: "same-origin" });
  if (!res.ok) throw new Error(`Download failed (${res.status})`);
  const blob = await res.blob();
  // Prefer RFC 5987 filename* (percent-encoded), fall back to plain filename.
  const disposition = res.headers.get("Content-Disposition") || "";
  const star = disposition.match(/filename\*=(?:UTF-8'')?"?([^";]+)"?/i);
  const plain = disposition.match(/filename="?([^";]+)"?/i);
  let name = star?.[1] || plain?.[1] || "";
  try {
    name = name ? decodeURIComponent(name) : "";
  } catch {
    // keep raw name
  }
  const objectUrl = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = objectUrl;
  a.download = name || fallbackName;
  a.click();
  window.setTimeout(() => URL.revokeObjectURL(objectUrl), 1000);
  // Safari expands its own toolbars for the download sheet and often fires
  // no resize event when the sheet closes; nudge a viewport + safe-area
  // re-measure so the tab bar keeps filling the bottom inset.
  for (const delay of [600, 1600]) {
    window.setTimeout(() => window.dispatchEvent(new Event("resize")), delay);
  }
}
