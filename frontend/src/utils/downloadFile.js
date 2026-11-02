// Saving files to disk. Everything that hands a file to the browser goes
// through here so the iOS behaviour and the Content-Disposition parsing stay in
// one place.
import { httpFetch } from "./http.js";

/**
 * `filename*` (RFC 5987, percent-encoded, used for non-ASCII names) first, then
 * the plain `filename`, then `fallback`.
 */
export function filenameFromDisposition(disposition, fallback = "download") {
  // `filename*` is percent-encoded by definition; the plain fallback is literal
  // ASCII (the server replaces non-ASCII with `_`), so decoding it would turn a
  // real `100%.jpg` into something else.
  const star = disposition.match(/filename\*=(?:UTF-8'')?"?([^";]+)"?/i);
  if (star?.[1]) {
    try {
      return decodeURIComponent(star[1]);
    } catch {
      return star[1];
    }
  }
  const plain = disposition.match(/filename="?([^";]+)"?/i);
  return plain?.[1] || fallback;
}

/**
 * Save a blob. On iOS a plain `<a href>` navigation hands the file to Safari's
 * inline viewer even when the response carries an attachment header; a blob URL
 * with the download attribute triggers the system download sheet instead. The
 * object URL is revoked later, not in the same tick: revoking right away can
 * cancel the save on slower devices.
 */
export function saveBlob(blob, filename = "download") {
  const objectUrl = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = objectUrl;
  anchor.download = filename;
  anchor.rel = "noopener";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(objectUrl), 1000);
  // Safari expands its own toolbars for the download sheet and often fires no
  // resize event when it closes; nudge a viewport + safe-area re-measure so the
  // tab bar keeps filling the bottom inset.
  for (const delay of [600, 1600]) {
    window.setTimeout(() => window.dispatchEvent(new Event("resize")), delay);
  }
}

/** Save a fetch response that carries the file (attachment). */
export async function saveBlobResponse(response, fallbackName = "download") {
  const blob = await response.blob();
  const disposition = response.headers.get("Content-Disposition") || "";
  saveBlob(blob, filenameFromDisposition(disposition, fallbackName));
}

/** Fetch a URL and hand it to the browser as a file. */
export async function downloadFileByUrl(url, fallbackName = "download") {
  const res = await httpFetch(url);
  if (!res.ok) {
    const payload = await res.json().catch(() => null);
    throw new Error(payload?.message || `Download failed (${res.status})`);
  }
  await saveBlobResponse(res, fallbackName);
}
