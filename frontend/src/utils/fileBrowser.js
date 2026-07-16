export function isImageEntry(entry) {
  if (!entry || entry.is_dir) {
    return false;
  }

  return /\.(jpe?g|png|gif|bmp|webp|tiff?|cr2|cr3|nef|arw|dng|orf|rw2|raf|pef|srw)$/i.test(entry.name || "");
}

export function thumbnailUrl(pathOrEntry, size = 256) {
  const path = typeof pathOrEntry === "string"
    ? pathOrEntry
    : pathOrEntry?.fullPath || "";
  return `/api/thumbnail/${encodeURIComponent(path)}?size=${size}`;
}