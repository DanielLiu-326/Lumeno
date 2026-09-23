export const COLOR_LABELS = [
  { key: "red", color: "bg-red-500", ring: "ring-red-400" },
  { key: "yellow", color: "bg-yellow-400", ring: "ring-yellow-300" },
  { key: "green", color: "bg-emerald-500", ring: "ring-emerald-400" },
  { key: "blue", color: "bg-blue-500", ring: "ring-blue-400" },
  { key: "purple", color: "bg-purple-500", ring: "ring-purple-400" },
];

export const PHOTO_FIELDS = [
  { key: "name", labelKey: "view.fieldName" },
  { key: "rating", labelKey: "view.fieldRating" },
  { key: "tags", labelKey: "view.fieldTags" },
  { key: "datetime", labelKey: "view.fieldDatetime" },
  { key: "camera", labelKey: "view.fieldCamera" },
  { key: "color", labelKey: "view.fieldColor" },
  { key: "lens", labelKey: "view.fieldLens" },
  { key: "size", labelKey: "view.fieldSize" },
];

export const FIELD_KEYS = PHOTO_FIELDS.map((field) => field.key);

export const OVERLAY_SLOTS = [
  { id: "overlay-tl", corner: "tl", labelKey: "view.overlayTl" },
  { id: "overlay-tr", corner: "tr", labelKey: "view.overlayTr" },
  { id: "overlay-bl", corner: "bl", labelKey: "view.overlayBl" },
  { id: "overlay-br", corner: "br", labelKey: "view.overlayBr" },
];

export const MAX_BELOW_ROWS = 6;
export const THUMB_SMALL_EXTRA = 4;
export const THUMB_DETAILS_MIN = 4;
export const THUMB_DEFAULT = 6;
export const THUMB_MAX = 9;
export const VIEW_LAYOUT_VERSION = 2;

export const PREVIEW_PHOTO = {
  name: "PANA5801.JPG",
  rating: 4,
  tags: ["旅行", "夜景", "城市"],
  datetime: "2026-08-12 19:42",
  date: "2026-08-12",
  camera_model: "DC-S5M2",
  color_label: ["red", "yellow"],
  lens: "Lumix 24-70",
  size: 24567890,
};

export function defaultViewLayout() {
  return {
    version: VIEW_LAYOUT_VERSION,
    thumbnailSize: THUMB_DEFAULT,
    compact: false,
    slots: {
      "overlay-tl": ["color"],
      "overlay-tr": [],
      "overlay-bl": ["rating"],
      "overlay-br": [],
    },
    rows: [
      ["name"],
      ["datetime", "camera"],
      ["tags"],
    ],
  };
}

export function normalizeColorLabels(value) {
  const labels = Array.isArray(value) ? value : String(value || "").split(",");
  return labels.map((label) => String(label).trim()).filter(Boolean);
}

function sanitizeFields(list) {
  const seen = new Set();
  const next = [];
  for (const key of Array.isArray(list) ? list : []) {
    if (!FIELD_KEYS.includes(key) || seen.has(key)) continue;
    seen.add(key);
    next.push(key);
  }
  return next;
}

export function normalizeViewLayout(raw) {
  const fallback = defaultViewLayout();
  const source = raw && typeof raw === "object" ? raw : {};
  const slots = {};
  const claimed = new Set();

  for (const slot of OVERLAY_SLOTS) {
    const fields = sanitizeFields(source.slots?.[slot.id]).filter((key) => {
      if (claimed.has(key)) return false;
      claimed.add(key);
      return true;
    });
    slots[slot.id] = fields;
  }

  const rows = [];
  for (const row of Array.isArray(source.rows) ? source.rows : fallback.rows) {
    if (rows.length >= MAX_BELOW_ROWS) break;
    const fields = sanitizeFields(row).filter((key) => {
      if (claimed.has(key)) return false;
      claimed.add(key);
      return true;
    });
    rows.push(fields);
  }

  let size = Number(source.thumbnailSize);
  const srcVer = Number(source.version) || 0;
  if (srcVer < 2 && Number.isFinite(size)) {
    size += THUMB_SMALL_EXTRA;
  }
  return {
    version: VIEW_LAYOUT_VERSION,
    thumbnailSize: Number.isFinite(size) ? Math.min(THUMB_MAX, Math.max(0, Math.round(size))) : THUMB_DEFAULT,
    compact: !!source.compact,
    slots,
    rows,
  };
}

export function cloneLayout(layout) {
  return normalizeViewLayout(layout);
}

export function placedFields(layout) {
  const used = new Set();
  for (const slot of OVERLAY_SLOTS) {
    for (const key of layout.slots?.[slot.id] || []) used.add(key);
  }
  for (const row of layout.rows || []) {
    for (const key of row) used.add(key);
  }
  return used;
}

export function unusedFields(layout) {
  const used = placedFields(layout);
  return PHOTO_FIELDS.filter((field) => !used.has(field.key));
}

export function hasBelowInfo(layout) {
  return (layout?.rows || []).some((row) => row.length > 0);
}

export function removeField(layout, key) {
  for (const slot of OVERLAY_SLOTS) {
    layout.slots[slot.id] = (layout.slots[slot.id] || []).filter((item) => item !== key);
  }
  layout.rows = (layout.rows || []).map((row) => row.filter((item) => item !== key));
}

export function addFieldToSlot(layout, slotId, key) {
  if (!FIELD_KEYS.includes(key) || !layout.slots[slotId]) return;
  removeField(layout, key);
  layout.slots[slotId].push(key);
}

export function addFieldToRow(layout, rowIndex, key) {
  if (!FIELD_KEYS.includes(key) || rowIndex < 0) return;
  removeField(layout, key);
  while (layout.rows.length <= rowIndex && layout.rows.length < MAX_BELOW_ROWS) {
    layout.rows.push([]);
  }
  if (!layout.rows[rowIndex]) return;
  layout.rows[rowIndex].push(key);
}

function storageKey(user) {
  return user ? `album-view-layout-v1:${user}` : "album-view-layout-v1";
}

function mobileCompactKey(user) {
  return user ? `album-view-compact-mobile:${user}` : "album-view-compact-mobile";
}

function readMobileCompact(user) {
  try {
    const raw = localStorage.getItem(mobileCompactKey(user));
    if (raw == null) return null;
    return raw === "1" || raw === "true";
  } catch {
    return null;
  }
}

export function loadViewLayout(user, { mobile = false } = {}) {
  try {
    const raw = localStorage.getItem(storageKey(user));
    const layout = raw ? normalizeViewLayout(JSON.parse(raw)) : defaultViewLayout();
    if (mobile) layout.compact = readMobileCompact(user) ?? true;
    return layout;
  } catch {
    const layout = defaultViewLayout();
    if (mobile) layout.compact = true;
    return layout;
  }
}

export function saveViewLayout(user, layout, { mobile = false } = {}) {
  try {
    const normalized = normalizeViewLayout(layout);
    if (mobile) {
      localStorage.setItem(mobileCompactKey(user), normalized.compact ? "1" : "0");
      const raw = localStorage.getItem(storageKey(user));
      if (raw) {
        try {
          normalized.compact = !!JSON.parse(raw).compact;
        } catch {
          normalized.compact = false;
        }
      } else {
        normalized.compact = false;
      }
    }
    localStorage.setItem(storageKey(user), JSON.stringify(normalized));
  } catch {
    /* ignore quota / private mode */
  }
}
