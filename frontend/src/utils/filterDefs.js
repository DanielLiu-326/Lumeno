// ─────────────────────────────────────────────────────────────────────────────
// Filter Registry (frontend)
//
// Each entry is the single source of truth for one filter type:
//   key            — unique identifier matching item.type / backend kind key
//   labelKey       — i18n key for the display name
//   defaultValue() — factory for a fresh, empty value object
//   isEmpty(v)     — true when the value carries no effective constraint
//   toWire(v)      — optional normalizer for the JSON value sent to the backend
//
// Logic (and / or / and_not) lives on each filter *item* and is sent to the
// backend as-is. The server evaluates left-to-right:
//   first clause is the seed (and_not negates it; and/or are the predicate);
//   later clauses apply acc ∧ P, acc ∨ P, or acc ∧ ¬P.
// Row order matters. AI text is still applied by the client as an intersection.
// ─────────────────────────────────────────────────────────────────────────────

const DATE_RANGE = {
  key: "date_range",
  labelKey: "filter.dateRange",
  defaultValue: () => ({ from: "", to: "", mode: "picker" }),
  isEmpty: (v) => !v?.from && !v?.to,
};

const RATING = {
  key: "rating",
  labelKey: "filter.rating",
  defaultValue: () => ({ ratings: [] }),
  isEmpty: (v) => !v?.ratings || v.ratings.length === 0,
};

const COLOR_LABEL = {
  key: "color_label",
  labelKey: "filter.colorLabel",
  defaultValue: () => ({ colorLabels: [] }),
  isEmpty: (v) => !v?.colorLabels || v.colorLabels.length === 0,
};

const FORMAT = {
  key: "format",
  labelKey: "filter.format",
  defaultValue: () => ({ format: "" }),
  isEmpty: (v) => !v?.format,
  toWire: (v) => ({ format: String(v.format || "").toLowerCase() }),
};

const TAGS = {
  key: "tags",
  labelKey: "filter.tags",
  defaultValue: () => ({ tags: "" }),
  isEmpty: (v) => !v?.tags,
};

const CAMERA = {
  key: "camera",
  labelKey: "filter.camera",
  defaultValue: () => ({ camera: "" }),
  isEmpty: (v) => !v?.camera,
};

const LENS = {
  key: "lens",
  labelKey: "filter.lens",
  defaultValue: () => ({ lens: "" }),
  isEmpty: (v) => !v?.lens,
};

const TEXT = {
  key: "text",
  labelKey: "filter.text",
  defaultValue: () => ({ q: "", minScore: 0 }),
  isEmpty: (v) => !String(v?.q || "").trim(),
  toWire: (v) => ({
    q: String(v.q || "").trim(),
    minScore: Number(v.minScore) || 0,
  }),
};

const PERSON = {
  key: "person",
  labelKey: "filter.person",
  defaultValue: () => ({ personIds: [] }),
  isEmpty: (v) => !Array.isArray(v?.personIds) || v.personIds.length === 0,
};

import FilterItemDateRange from "../components/filters/FilterItemDateRange.vue";
import FilterItemRating from "../components/filters/FilterItemRating.vue";
import FilterItemColorLabel from "../components/filters/FilterItemColorLabel.vue";
import FilterItemFormat from "../components/filters/FilterItemFormat.vue";
import FilterItemTags from "../components/filters/FilterItemTags.vue";
import FilterItemCamera from "../components/filters/FilterItemCamera.vue";
import FilterItemLens from "../components/filters/FilterItemLens.vue";
import FilterItemText from "../components/filters/FilterItemText.vue";
import FilterItemPerson from "../components/filters/FilterItemPerson.vue";

DATE_RANGE.component = FilterItemDateRange;
DATE_RANGE.defaultEnabled = true;
RATING.component = FilterItemRating;
RATING.defaultEnabled = true;
COLOR_LABEL.component = FilterItemColorLabel;
COLOR_LABEL.defaultEnabled = true;
FORMAT.component = FilterItemFormat;
FORMAT.defaultEnabled = true;
TAGS.component = FilterItemTags;
TAGS.defaultEnabled = true;
CAMERA.component = FilterItemCamera;
CAMERA.defaultEnabled = false;
LENS.component = FilterItemLens;
LENS.defaultEnabled = false;
TEXT.component = FilterItemText;
TEXT.defaultEnabled = true;
PERSON.component = FilterItemPerson;
PERSON.defaultEnabled = false;

/** Ordered list of all filter definitions (drives the "Add filter" menu). */
export const FILTER_REGISTRY = [DATE_RANGE, RATING, COLOR_LABEL, FORMAT, TAGS, TEXT, PERSON, CAMERA, LENS];

/** Multi-level album sort fields (labelKey is resolved at render time). */
export const SORT_FIELDS = [
  { key: "date", labelKey: "albums.sortDate" },
  { key: "name", labelKey: "albums.sortName" },
  { key: "size", labelKey: "albums.sortSize" },
  { key: "rating", labelKey: "albums.sortRating" },
  { key: "camera", labelKey: "albums.sortCamera" },
  { key: "lens", labelKey: "albums.sortLens" },
];

export function nextSortLevel(sort) {
  const used = (sort || []).map((s) => s.field);
  return SORT_FIELDS.find((f) => !used.includes(f.key)) || null;
}

export function withAddedSortLevel(sort) {
  const next = nextSortLevel(sort);
  if (!next) return sort || [];
  return [...(sort || []), { field: next.key, dir: "desc" }];
}

export function withAddedSortField(sort, field, dir = "desc") {
  if (!field || (sort || []).some((s) => s.field === field)) return sort || [];
  return [...(sort || []), { field, dir }];
}

export function withRemovedSortLevel(sort, index) {
  const next = [...(sort || [])];
  next.splice(index, 1);
  return next;
}

export function withSortField(sort, index, field) {
  return (sort || []).map((level, i) => (i === index ? { ...level, field } : level));
}

export function withToggledSortDir(sort, index) {
  return (sort || []).map((level, i) =>
    i === index ? { ...level, dir: level.dir === "asc" ? "desc" : "asc" } : level,
  );
}

/** Map from key → definition for O(1) lookup. */
export const FILTER_DEF_MAP = Object.fromEntries(FILTER_REGISTRY.map((d) => [d.key, d]));

/** Default filter rows shown when no saved state exists for an album. */
export function defaultFilterItems() {
  return FILTER_REGISTRY.filter((def) => def.defaultEnabled).map((def) => ({
    type: def.key,
    logic: "and",
    enabled: true,
    value: def.defaultValue(),
  }));
}

export function filterStorageKey(user, album) {
  return `album_filters_v1_${user}_${album}`;
}

/** Load saved filter rows for an album (or defaults). */
export function loadFilterItemsFromStorage(user, album) {
  if (!album) return defaultFilterItems();
  try {
    const saved = localStorage.getItem(filterStorageKey(user, album));
    if (saved) {
      const parsed = JSON.parse(saved);
      if (Array.isArray(parsed) && parsed.length > 0) {
        return parsed;
      }
    }
  } catch {
    /* ignore corrupt storage */
  }
  return defaultFilterItems();
}

/** Compile UI filter rows (+ optional collection constraints) into API clauses. */
export function compileAlbumFilterClauses(filterItems, collectionFilter = {}) {
  return buildFilterClauses(filterItems, collectionFilter);
}

/**
 * Build wire clauses for POST /api/albums/photos.
 * @param {Array} filterItems — UI filter rows { type, logic, enabled, value }
 * @param {object} collectionFilter — active collection base constraints
 */
export function buildFilterClauses(filterItems, collectionFilter = {}) {
  const clauses = [];
  const cf = collectionFilter || {};

  for (const item of filterItems || []) {
    if (!item?.enabled) continue;
    const def = FILTER_DEF_MAP[item.type];
    if (!def || def.isEmpty(item.value)) continue;
    const value = def.toWire ? def.toWire(item.value) : { ...item.value };
    clauses.push({
      type: item.type,
      logic: item.logic || "and",
      enabled: true,
      value,
    });
  }

  // Collection scope is AND'd onto the whole user chain (must come last).
  if (cf.from || cf.to) {
    clauses.push({
      type: "date_range",
      logic: "and",
      enabled: true,
      value: { from: cf.from || "", to: cf.to || "" },
    });
  }
  if (cf.ratings) {
    const ratings = String(cf.ratings)
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean)
      .map(Number)
      .filter((n) => Number.isFinite(n));
    if (ratings.length) {
      clauses.push({
        type: "rating",
        logic: "and",
        enabled: true,
        value: { ratings },
      });
    }
  }
  if (Array.isArray(cf.color_label) && cf.color_label.length) {
    clauses.push({
      type: "color_label",
      logic: "and",
      enabled: true,
      value: { colorLabels: [...cf.color_label] },
    });
  }
  if (cf.format) {
    clauses.push({
      type: "format",
      logic: "and",
      enabled: true,
      value: { format: String(cf.format).toLowerCase() },
    });
  }
  if (cf.camera) {
    clauses.push({
      type: "camera",
      logic: "and",
      enabled: true,
      value: { camera: cf.camera },
    });
  }
  if (cf.lens) {
    clauses.push({
      type: "lens",
      logic: "and",
      enabled: true,
      value: { lens: cf.lens },
    });
  }
  if (cf.tags) {
    clauses.push({
      type: "tags",
      logic: "and",
      enabled: true,
      value: { tags: cf.tags },
    });
  }

  return clauses;
}

/** Split AI semantic text clauses from metadata clauses. */
export function partitionFilterClauses(clauses) {
  const list = Array.isArray(clauses) ? clauses : [];
  const aiText = [];
  const metadata = [];
  for (const clause of list) {
    if (clause?.type === "text") aiText.push(clause);
    else metadata.push(clause);
  }
  return { aiText, metadata };
}
