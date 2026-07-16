// ─────────────────────────────────────────────────────────────────────────────
// Filter Registry
//
// Each entry is the single source of truth for one filter type:
//   key         — unique identifier matching the stored item.type
//   labelKey    — i18n key for the display name
//   defaultValue() — factory for a fresh, empty value object
//   isEmpty(v)  — true when the value carries no effective constraint
//   compile(and, or, andNot, collectionBase, avail) → partial result fields
//
// compile() receives:
//   and / or / andNot — filter items (already typed to this def) partitioned
//                       by their logic field
//   collectionBase    — collectionFilter.value (the active collection's filter)
//   avail             — { availableFormats, availableCameras, availableLenses }
//
// compile() returns an object that is merged into the compiled result:
//   { from, to }          — date range
//   { ratings }           — number[]
//   { colorLabels }       — string[]
//   { formats }           — string[] | null  (null = no constraint)
//   { cameras }           — string[] | null
//   { lenses }            — string[] | null
//   { tags }              — string (comma-separated)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Compute the final allowlist from AND / OR / AND-NOT item value-arrays.
 *
 * Semantics:
 *   AND sets   → intersect  (result must satisfy every AND item that has values)
 *   OR  values → union with AND result
 *   NOT values → subtract from result
 *
 * If there are only NOT items (no AND/OR), the universe is used as the base set
 * so that exclusion has meaning.
 *
 * Returns null when there is no effective constraint at all.
 * Returns [] when constraints are contradictory (nothing can match).
 *
 * @param {Array<string[]>} andSets   — one array of allowed values per AND item
 * @param {string[]}        orVals    — values from OR items (already flat)
 * @param {string[]}        notVals   — values from AND-NOT items (already flat)
 * @param {string[] | null} universe  — full possible value set for NOT-only case
 */
function resolveAllowlist(andSets, orVals, notVals, universe) {
  const hasAnd = andSets.some((s) => s.length > 0);
  const hasOr = orVals.length > 0;
  const hasNot = notVals.length > 0;

  if (!hasAnd && !hasOr && !hasNot) return null; // no constraint

  // Intersect non-empty AND sets
  let andResult = null;
  for (const set of andSets) {
    if (set.length === 0) continue; // empty item means "no selection" → skip
    andResult = andResult === null ? [...set] : andResult.filter((v) => set.includes(v));
  }

  // Union with OR values
  const combined = andResult !== null ? [...andResult] : [];
  for (const v of orVals) {
    if (!combined.includes(v)) combined.push(v);
  }

  // If we only have NOT items (nothing in AND/OR), start from universe
  if (!hasAnd && !hasOr && hasNot) {
    if (!universe || universe.length === 0) return null;
    combined.push(...universe.filter((v) => !combined.includes(v)));
  }

  return hasNot ? combined.filter((v) => !notVals.includes(v)) : combined;
}

// ── Individual filter definitions ─────────────────────────────────────────────

const DATE_RANGE = {
  key: "date_range",
  labelKey: "filter.dateRange",
  defaultValue: () => ({ from: "", to: "", mode: "picker" }),
  isEmpty: (v) => !v.from && !v.to,
  /**
   * AND: first non-empty from / to wins (narrows).
   * OR / AND-NOT: not representable with a single [from, to] interval — ignored.
   */
  compile(and, _or, _andNot, base) {
    const r = {};
    if (base.from) r.from = base.from;
    if (base.to) r.to = base.to;
    for (const item of and) {
      if (item.value.from && !r.from) r.from = item.value.from;
      if (item.value.to && !r.to) r.to = item.value.to;
    }
    return r;
  },
};

const RATING = {
  key: "rating",
  labelKey: "filter.rating",
  defaultValue: () => ({ ratings: [] }),
  isEmpty: (v) => !v.ratings || v.ratings.length === 0,
  compile(and, or, andNot, base) {
    const universe = ["0", "1", "2", "3", "4", "5"];
    const andSets = and.map((i) => (i.value.ratings || []).map(String));
    const orVals = or.flatMap((i) => (i.value.ratings || []).map(String));
    const notVals = andNot.flatMap((i) => (i.value.ratings || []).map(String));

    const fromItems = resolveAllowlist(andSets, orVals, notVals, universe);

    const baseRatings = base.ratings
      ? String(base.ratings).split(",").map((s) => s.trim()).filter(Boolean)
      : [];

    let merged;
    if (fromItems === null) {
      merged = baseRatings;
    } else if (baseRatings.length > 0) {
      merged = baseRatings.filter((r) => fromItems.includes(r));
    } else {
      merged = fromItems;
    }

    return { ratings: merged.map(Number).sort((a, b) => a - b) };
  },
};

const COLOR_LABEL = {
  key: "color_label",
  labelKey: "filter.colorLabel",
  defaultValue: () => ({ colorLabels: [] }),
  isEmpty: (v) => !v.colorLabels || v.colorLabels.length === 0,
  compile(and, or, andNot, base) {
    const universe = ["red", "yellow", "green", "blue", "purple"];
    const andSets = and.map((i) => (i.value.colorLabels || []));
    const orVals = or.flatMap((i) => (i.value.colorLabels || []));
    const notVals = andNot.flatMap((i) => (i.value.colorLabels || []));

    const fromItems = resolveAllowlist(andSets, orVals, notVals, universe);

    const baseColors = Array.isArray(base.color_label) ? base.color_label : [];

    let merged;
    if (fromItems === null) {
      merged = baseColors;
    } else if (baseColors.length > 0) {
      merged = baseColors.filter((c) => fromItems.includes(c));
    } else {
      merged = fromItems;
    }

    return { colorLabels: merged };
  },
};

const FORMAT = {
  key: "format",
  labelKey: "filter.format",
  defaultValue: () => ({ format: "" }),
  isEmpty: (v) => !v.format,
  /**
   * Format values are lowercased file extensions (e.g. "jpg", "raw").
   * AND: intersect allowed sets.
   * OR:  union with AND result.
   * AND-NOT: subtract from result, using availableFormats as universe when
   *          there are no AND/OR constraints (so AND NOT "jpg" ≡ "everything except jpg").
   */
  compile(and, or, andNot, base, avail) {
    const universe = (avail.availableFormats || []).map((f) => f.toLowerCase());
    const andSets = and.map((i) => (i.value.format ? [i.value.format.toLowerCase()] : []));
    const orVals = or.map((i) => i.value.format?.toLowerCase()).filter(Boolean);
    const notVals = andNot.map((i) => i.value.format?.toLowerCase()).filter(Boolean);

    const baseFormat = base.format ? base.format.toLowerCase() : null;
    // When resolving, the universe for NOT-only case is: baseFormat or availableFormats
    const effectiveUniverse = baseFormat ? [baseFormat] : (universe.length > 0 ? universe : null);
    const fromItems = resolveAllowlist(andSets, orVals, notVals, effectiveUniverse);

    let merged;
    if (fromItems === null) {
      // No item-level constraint — use collection base only
      merged = baseFormat ? [baseFormat] : null;
    } else if (baseFormat !== null) {
      // Intersect item result with collection base
      merged = fromItems.includes(baseFormat) ? fromItems : [];
    } else {
      merged = fromItems;
    }

    return { formats: merged }; // null = no constraint; [] = nothing matches
  },
};

const TAGS = {
  key: "tags",
  labelKey: "filter.tags",
  defaultValue: () => ({ tags: "" }),
  isEmpty: (v) => !v.tags,
  /**
   * The backend requires ALL specified tags (AND semantics within the tag list).
   * AND items: accumulate all tags — photo must carry every one.
   * OR / AND-NOT: not representable with the current flat backend API — ignored.
   */
  compile(and, _or, _andNot, base) {
    const tagSet = new Set(
      base.tags ? base.tags.split(",").map((s) => s.trim()).filter(Boolean) : [],
    );
    for (const item of and) {
      for (const tag of (item.value.tags || "").split(",").map((s) => s.trim()).filter(Boolean)) {
        tagSet.add(tag);
      }
    }
    return { tags: [...tagSet].join(", ") };
  },
};

const CAMERA = {
  key: "camera",
  labelKey: "filter.camera",
  defaultValue: () => ({ camera: "" }),
  isEmpty: (v) => !v.camera,
  compile(and, or, andNot, base, avail) {
    const universe = (avail.availableCameras || []).map((c) => c.toLowerCase());
    const andSets = and.map((i) => (i.value.camera ? [i.value.camera.toLowerCase()] : []));
    const orVals = or.map((i) => i.value.camera?.toLowerCase()).filter(Boolean);
    const notVals = andNot.map((i) => i.value.camera?.toLowerCase()).filter(Boolean);

    const baseCamera = base.camera ? base.camera.toLowerCase() : null;
    const effectiveUniverse = baseCamera ? [baseCamera] : (universe.length > 0 ? universe : null);
    const fromItems = resolveAllowlist(andSets, orVals, notVals, effectiveUniverse);

    let merged;
    if (fromItems === null) {
      merged = baseCamera ? [baseCamera] : null;
    } else if (baseCamera !== null) {
      merged = fromItems.includes(baseCamera) ? fromItems : [];
    } else {
      merged = fromItems;
    }

    return { cameras: merged };
  },
};

const LENS = {
  key: "lens",
  labelKey: "filter.lens",
  defaultValue: () => ({ lens: "" }),
  isEmpty: (v) => !v.lens,
  compile(and, or, andNot, base, avail) {
    const universe = (avail.availableLenses || []).map((l) => l.toLowerCase());
    const andSets = and.map((i) => (i.value.lens ? [i.value.lens.toLowerCase()] : []));
    const orVals = or.map((i) => i.value.lens?.toLowerCase()).filter(Boolean);
    const notVals = andNot.map((i) => i.value.lens?.toLowerCase()).filter(Boolean);

    const baseLens = base.lens ? base.lens.toLowerCase() : null;
    const effectiveUniverse = baseLens ? [baseLens] : (universe.length > 0 ? universe : null);
    const fromItems = resolveAllowlist(andSets, orVals, notVals, effectiveUniverse);

    let merged;
    if (fromItems === null) {
      merged = baseLens ? [baseLens] : null;
    } else if (baseLens !== null) {
      merged = fromItems.includes(baseLens) ? fromItems : [];
    } else {
      merged = fromItems;
    }

    return { lenses: merged };
  },
};

// ── Vue components (one per filter type) ─────────────────────────────────────
// Imported here so the registry is the single place that couples a type key
// to its UI. FilterSidebar never references specific filter types by name.
import FilterItemDateRange from "../components/filters/FilterItemDateRange.vue";
import FilterItemRating from "../components/filters/FilterItemRating.vue";
import FilterItemColorLabel from "../components/filters/FilterItemColorLabel.vue";
import FilterItemFormat from "../components/filters/FilterItemFormat.vue";
import FilterItemTags from "../components/filters/FilterItemTags.vue";
import FilterItemCamera from "../components/filters/FilterItemCamera.vue";
import FilterItemLens from "../components/filters/FilterItemLens.vue";

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

// ── Exports ───────────────────────────────────────────────────────────────────

/** Ordered list of all filter definitions (drives the "Add filter" menu). */
export const FILTER_REGISTRY = [DATE_RANGE, RATING, COLOR_LABEL, FORMAT, TAGS, CAMERA, LENS];

/** Map from key → definition for O(1) lookup. */
export const FILTER_DEF_MAP = Object.fromEntries(FILTER_REGISTRY.map((d) => [d.key, d]));
