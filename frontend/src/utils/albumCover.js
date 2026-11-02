import { DEFAULT_THUMBNAIL_SIZE, thumbnailUrl } from "./thumbnailUrl.js";

/** Owner of a shared album, from `owner` or `owner/dir` access keys. */
export function albumOwnerName(album, fallback = "") {
  if (album?.owner) return String(album.owner);
  const key = String(album?.dir_name || album?.album || "");
  const slash = key.indexOf("/");
  if (slash > 0) return key.slice(0, slash);
  return fallback || "";
}

export function albumCoverUrl(album, size = DEFAULT_THUMBNAIL_SIZE, fallbackOwner = "") {
  if (!album?.cover) return "";
  const owner = albumOwnerName(album, fallbackOwner);
  return thumbnailUrl(album.cover, { size, owner });
}
