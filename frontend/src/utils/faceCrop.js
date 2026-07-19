/**
 * CSS crop for a normalized face box `[x, y, w, h]` in `[0, 1]`.
 * Parent must be `position: relative; overflow: hidden`.
 */
export function faceCropStyle(box) {
  if (!Array.isArray(box) || box.length < 4) {
    return { objectFit: "cover", objectPosition: "center" };
  }
  const [x, y, w, h] = box.map(Number);
  // Expect normalized [0,1] xywh. Pixel boxes break CSS crop — fall back to center.
  if (
    ![x, y, w, h].every((value) => Number.isFinite(value))
    || w <= 0
    || h <= 0
    || Math.max(x, y, w, h) > 1.5
  ) {
    return { objectFit: "cover", objectPosition: "center" };
  }
  const pad = 0.15;
  const left = Math.max(0, x - w * pad);
  const top = Math.max(0, y - h * pad);
  const width = Math.min(1 - left, w * (1 + pad * 2));
  const height = Math.min(1 - top, h * (1 + pad * 2));
  if (width <= 0.01 || height <= 0.01) {
    return { objectFit: "cover", objectPosition: "center" };
  }
  return {
    position: "absolute",
    width: `${100 / width}%`,
    height: `${100 / height}%`,
    left: `${(-left / width) * 100}%`,
    top: `${(-top / height) * 100}%`,
    maxWidth: "none",
  };
}

export function personCoverThumbnailUrl(cover, size = 360) {
  if (!cover?.path) return "";
  const owner = cover.owner ? `&owner=${encodeURIComponent(cover.owner)}` : "";
  return `/api/thumbnail/${encodeURIComponent(cover.path)}?size=${size}${owner}`;
}
