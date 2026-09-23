/** Padding around the face box when cropping avatars (matches backend). */
export const FACE_CROP_PAD = 0.15;

/**
 * CSS crop for a normalized face box `[x, y, w, h]` in `[0, 1]`.
 * Parent must be `position: relative; overflow: hidden`.
 * @deprecated Prefer server-side `/api/ai/albums/.../face-crop`.
 */
export function faceCropStyle(box) {
  if (!Array.isArray(box) || box.length < 4) {
    return { objectFit: "cover", objectPosition: "center" };
  }
  const [x, y, w, h] = box.map(Number);
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

/** Album param for face-crop (`album` or shared `owner/album`). */
export function resolveFaceCropAlbum(albumName = "") {
  return String(albumName || "").trim();
}

/**
 * Folder under the owner's `albums/` directory.
 * Shared albums are addressed as `owner/album`; photo files live at `albums/album/…`.
 */
export function albumStorageName(albumKey = "") {
  const key = String(albumKey || "").replace(/\\/g, "/").trim().replace(/^\/+|\/+$/g, "");
  const slash = key.indexOf("/");
  if (slash > 0 && slash < key.length - 1) return key.slice(slash + 1);
  return key;
}

/** Parse `albums/{album}/{relative}` photo API paths. */
export function parseAlbumPhotoPath(path) {
  const normalized = String(path || "").replace(/\\/g, "/").replace(/^\/+/, "");
  const match = normalized.match(/^albums\/([^/]+)\/(.+)$/);
  if (!match) return null;
  return { album: match[1], source: match[2] };
}

export function albumFaceCropUrl(albumName, faceId) {
  const album = resolveFaceCropAlbum(albumName);
  if (!album || !faceId) return "";
  const params = new URLSearchParams({ face_id: faceId });
  return `/api/ai/albums/${encodeURIComponent(album)}/face-crop?${params.toString()}`;
}

export function albumFaceCropUrlByBox(albumName, source, boxXywh) {
  const album = resolveFaceCropAlbum(albumName);
  if (!album || !source || !isValidFaceBox(boxXywh)) return "";
  const box = boxXywh.map((v) => Number(v)).join(",");
  const params = new URLSearchParams({ source, box });
  return `/api/ai/albums/${encodeURIComponent(album)}/face-crop?${params.toString()}`;
}

export const FACE_AVATAR_REQUEST_SIZE = 256;

/** @deprecated display/dpr ignored — always {@link FACE_AVATAR_REQUEST_SIZE}. */
export function faceAvatarRequestSize(_displayPx, _dpr = 1) {
  return FACE_AVATAR_REQUEST_SIZE;
}

/**
 * Build face-crop URL from a cover/appearance-like object.
 * Prefers `face_id`, else `source` + `face_box` / `box_xywh`.
 */
export function personFaceCropUrl(subject, albumName = "") {
  if (!subject) return "";
  const album = resolveFaceCropAlbum(subject.album || albumName);
  if (!album) return "";

  const faceId = subject.face_id || subject.faceId;
  if (faceId) return albumFaceCropUrl(album, faceId);

  const box = subject.face_box || subject.box_xywh || subject.box;
  let source = subject.source;
  if (!source && subject.path) {
    source = parseAlbumPhotoPath(subject.path)?.source || "";
  }
  if (source && isValidFaceBox(box)) {
    return albumFaceCropUrlByBox(album, source, box);
  }
  return "";
}

/** Build API photo path for an album-relative source. */
export function albumPhotoApiPath(albumName, relativeSource) {
  if (!relativeSource) return "";
  const rel = String(relativeSource).replace(/\\/g, "/").replace(/^\/+/, "");
  if (!rel) return "";
  if (rel.startsWith("albums/")) return rel;
  const folder = albumStorageName(albumName);
  if (!folder) return rel;
  return `albums/${folder}/${rel}`;
}

/** Distinct photos a person appears in (filter / list “N photos”). */
export function personPhotoCount(person) {
  if (!person || typeof person !== "object") return 0;
  const visible = Number(person.visible_photo_count);
  if (Number.isFinite(visible) && visible >= 0 && person.visible_photo_count != null) {
    return visible;
  }
  const appearances = Array.isArray(person.appearances) ? person.appearances : [];
  if (appearances.length) {
    return new Set(appearances.map((row) => row?.source).filter(Boolean)).size;
  }
  const n = Number(person.appearance_count);
  return Number.isFinite(n) && n >= 0 ? n : 0;
}

/** Cover for UI lists: explicit cover, else first appearance fallback. */
export function personDisplayCover(person, albumName = "", owner = "") {
  const album = resolveFaceCropAlbum(albumName);
  if (person?.cover) {
    return {
      ...person.cover,
      album,
      face_id: person.cover.face_id,
      source: person.cover.source || parseAlbumPhotoPath(person.cover.path)?.source,
      face_box: person.cover.face_box ?? person.cover.box_xywh,
      owner: person.cover.owner || owner || person?.owner || "",
    };
  }
  const first = person?.appearances?.[0];
  if (!first?.source) return null;
  return {
    album,
    face_id: first.face_id,
    source: first.source,
    face_box: first.box_xywh,
    owner: owner || person?.owner || "",
  };
}
