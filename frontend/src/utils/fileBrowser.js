import { isVideoPhoto } from "./mediaKind.js";
import { thumbnailUrl as buildThumbnailUrl } from "./thumbnailUrl.js";

export function isImageEntry(entry) {
  if (!entry || entry.is_dir) {
    return false;
  }

  return /\.(jpe?g|png|gif|bmp|webp|tiff?|cr2|cr3|nef|arw|dng|orf|rw2|raf|pef|srw)$/i.test(entry.name || "");
}

export function hasThumbnail(entry) {
  return isImageEntry(entry) || isVideoPhoto(entry);
}

/** File-browser rows are small; the shared builder keeps the URL format. */
export const FILE_BROWSER_THUMBNAIL_SIZE = 256;

export function thumbnailUrl(pathOrEntry, size = FILE_BROWSER_THUMBNAIL_SIZE) {
  const path = typeof pathOrEntry === "string"
    ? pathOrEntry
    : pathOrEntry?.fullPath || "";
  return buildThumbnailUrl(path, { size });
}