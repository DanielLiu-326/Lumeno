/** Padding around the face box when cropping avatars (matches backend). */
export const FACE_CROP_PAD = 0.15;

/**
 * CSS crop for a normalized face box `[x, y, w, h]` in `[0, 1]`.
 * Parent must be `position: relative; overflow: hidden`.
 * Prefer `faceThumbnailUrl` when available — it crops from full resolution.
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
  const left = Math.max(0, x - w * FACE_CROP_PAD);
  const top = Math.max(0, y - h * FACE_CROP_PAD);
  const width = Math.min(1 - left, w * (1 + FACE_CROP_PAD * 2));
  const height = Math.min(1 - top, h * (1 + FACE_CROP_PAD * 2));
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

function isValidFaceBox(box) {
  if (!Array.isArray(box) || box.length < 4) return false;
  const [x, y, w, h] = box.map(Number);
  return [x, y, w, h].every((value) => Number.isFinite(value))
    && w > 0
    && h > 0
    && x >= 0
    && y >= 0
    && x + w <= 1.01
    && y + h <= 1.01;
}

/**
 * Request size for face avatar thumbnails. Scales with display size and DPR.
 * @param {number} displayPx CSS display size of the avatar
 * @param {number} [dpr=1] device pixel ratio
 */
export function faceAvatarRequestSize(displayPx, dpr = 1) {
  const base = Math.max(64, Math.round(Number(displayPx) * Number(dpr) * 2));
  return Math.min(512, base);
}

/**
 * Server-side face crop from full-resolution source (sharp even for small faces).
 */
export function faceThumbnailUrl(path, box, size = 256, owner = "") {
  if (!path || !isValidFaceBox(box)) return "";
  const [x, y, w, h] = box.map(Number);
  const boxParam = `${x},${y},${w},${h}`;
  const ownerQ = owner ? `&owner=${encodeURIComponent(owner)}` : "";
  return `/api/face-thumbnail/${encodeURIComponent(path)}?box=${encodeURIComponent(boxParam)}&size=${size}${ownerQ}`;
}

export function personCoverThumbnailUrl(cover, displayPx = 128) {
  if (!cover?.path) return "";
  const owner = cover.owner || "";
  const requestSize = faceAvatarRequestSize(displayPx);
  if (isValidFaceBox(cover.face_box)) {
    return faceThumbnailUrl(cover.path, cover.face_box, requestSize, owner);
  }
  const ownerQ = owner ? `&owner=${encodeURIComponent(owner)}` : "";
  return `/api/thumbnail/${encodeURIComponent(cover.path)}?size=${requestSize}${ownerQ}`;
}
