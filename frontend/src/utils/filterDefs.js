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
// backend as-is. The server evaluates:
//   (∀ AND) ∧ (OR为空 ∨ ∃ OR) ∧ (∀ AND_NOT 不成立)
// across all filter types — no frontend flattening.
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

/** Map from key → definition for O(1) lookup. */
export const FILTER_DEF_MAP = Object.fromEntries(FILTER_REGISTRY.map((d) => [d.key, d]));

/**
 * Build wire clauses for POST /api/albums/photos.
 * @param {Array} filterItems — UI filter rows { type, logic, enabled, value }
 * @param {object} collectionFilter — active collection base constraints
 */
export function buildFilterClauses(filterItems, collectionFilter = {}) {
  const clauses = [];
  const cf = collectionFilter || {};

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
