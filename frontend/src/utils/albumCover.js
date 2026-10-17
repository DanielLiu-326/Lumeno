import { thumbnailPathSegment } from "./mediaKind.js";

/** Owner of a shared album, from `owner` or `owner/dir` access keys. */
export function albumOwnerName(album, fallback = "") {
  if (album?.owner) return String(album.owner);
  const key = String(album?.dir_name || album?.album || "");
  const slash = key.indexOf("/");
  if (slash > 0) return key.slice(0, slash);
  return fallback || "";
}

export function albumCoverUrl(album, size = 400, fallbackOwner = "") {
  if (!album?.cover) return "";
  const owner = albumOwnerName(album, fallbackOwner);
  const ownerParam = owner ? `&owner=${encodeURIComponent(owner)}` : "";
  return `/api/thumbnail/${thumbnailPathSegment(album.cover)}?size=${size}${ownerParam}`;
}
