/** Normalize dashboard/discovery photo stubs for the album detail overlay. */
export function normalizeDiscoveryPhoto(photo) {
  if (!photo?.path) return null;
  const name = photo.name || String(photo.path).split("/").pop() || photo.path;
  return {
    ...photo,
    name,
    datetime: photo.datetime || photo.date || "",
    date: photo.date || "",
    tags: photo.tags || [],
    rating: photo.rating || 0,
    color_label: photo.color_label || [],
    width: photo.width || 0,
    height: photo.height || 0,
    size: photo.size ?? 0,
  };
}
