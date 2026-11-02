import { ref } from "vue";

const STORAGE_KEY = "album_graded_thumbs_v2";

/** Bumped when a grid thumb is permanently replaced, so open grids refetch it. */
export const gradedThumbEpoch = ref(0);

const revs = new Map();

function storageId(owner, path) {
  return `${owner || ""}|${path || ""}`;
}

function persist() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(Object.fromEntries(revs)));
  } catch {
    /* ignore quota / private mode */
  }
}

try {
  const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) || "{}");
  if (saved && typeof saved === "object") {
    for (const [key, rev] of Object.entries(saved)) {
      const n = Number(rev);
      if (key && Number.isFinite(n) && n > 0) revs.set(key, n);
    }
  }
} catch {
  /* ignore corrupt storage */
}

/** Cache-busting revision for a grid thumb, or 0 when it was never replaced. */
export function gradedThumbRev(owner, path) {
  return revs.get(storageId(owner, path)) || 0;
}

/** Remember that this photo's grid thumb is the darktable render. */
export function markGradedThumb(owner, path) {
  if (!path) return;
  const id = storageId(owner, path);
  if (revs.has(id)) return;
  revs.set(id, 1);
  persist();
  gradedThumbEpoch.value += 1;
}
