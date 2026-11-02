// One thumbnail URL builder for every surface (grid, covers, lists, mobile).
//
// The query string is built with URLSearchParams: hand-written `?owner=` /
// `&owner=` concatenation is what produced the two spellings in the first place.
import { isVideoPhoto } from "./mediaKind.js";
import { encodePath } from "./path.js";

/** Shared default edge length. Callers pass a size only when they really differ. */
export const DEFAULT_THUMBNAIL_SIZE = 360;

/**
 * `/api/thumbnail/{path}?size=…&owner=…`
 *
 * @param {string} path  path relative to the data root
 * @param {object} options
 * @param {number} options.size   edge length; `0` asks for the full-size render
 * @param {string} options.owner  album owner, omitted for the caller's own photos
 * @param {object} options.params extra query flags (e.g. `graded`)
 */
export function thumbnailUrl(path, { size = DEFAULT_THUMBNAIL_SIZE, owner = "", params = null } = {}) {
  if (!path) return "";
  // Mobile browsers refuse to paint an <img> whose URL ends in .mp4/.mov, so a
  // video poster gets a trailing .webp the server strips again.
  const segment = isVideoPhoto({ path }) ? `${encodePath(path)}.webp` : encodePath(path);
  const search = new URLSearchParams();
  search.set("size", String(size));
  if (owner) search.set("owner", String(owner));
  if (params) {
    for (const [key, value] of Object.entries(params)) {
      if (value === undefined || value === null || value === "") continue;
      search.set(key, String(value));
    }
  }
  return `/api/thumbnail/${segment}?${search.toString()}`;
}

/**
 * Thumbnail for a photo record (`{ path, owner }`), falling back to the album
 * owner when the record does not carry one.
 */
export function photoThumbnailUrl(photo, { size, owner, ...rest } = {}) {
  if (!photo?.path) return "";
  return thumbnailUrl(photo.path, {
    size: size ?? DEFAULT_THUMBNAIL_SIZE,
    owner: owner ?? photo.owner ?? "",
    ...rest,
  });
}
