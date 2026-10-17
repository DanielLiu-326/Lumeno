const VIDEO_EXTS = new Set(["mp4", "m4v", "mov", "webm"]);

export function isVideoPhoto(photo) {
  if (!photo) return false;
  const name = String(photo.name || photo.path || "");
  const ext = name.split(".").pop()?.toLowerCase() || "";
  return VIDEO_EXTS.has(ext);
}

/**
 * Path segment for `/api/thumbnail/{segment}`.
 * Mobile browsers refuse to paint an img whose URL ends in .mp4/.mov,
 * so video requests get a trailing .webp that the server strips.
 */
export function thumbnailPathSegment(path) {
  const encoded = encodeURIComponent(path || "");
  return isVideoPhoto({ path }) ? `${encoded}.webp` : encoded;
}

export function formatDurationMs(ms) {
  const n = Number(ms);
  if (!Number.isFinite(n) || n <= 0) return "";
  const total = Math.round(n / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (value) => String(value).padStart(2, "0");
  if (h > 0) return `${h}:${pad(m)}:${pad(s)}`;
  return `${m}:${pad(s)}`;
}

export function playbackUrl(path, owner) {
  const params = new URLSearchParams();
  if (owner) params.set("owner", owner);
  params.set("play", "1");
  return `/api/download/${encodeURIComponent(path)}?${params}`;
}
