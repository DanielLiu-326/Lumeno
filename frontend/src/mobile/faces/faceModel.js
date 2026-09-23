import { t } from "../../i18n/index.js";
import { albumPhotoApiPath } from "../../utils/faceCrop.js";

export function albumPhotoPath(albumName, relativeSource) {
  return albumPhotoApiPath(albumName, relativeSource);
}

export function photoThumb(path, owner) {
  if (!path) return "";
  const ownerParam = owner ? `&owner=${encodeURIComponent(owner)}` : "";
  return `/api/thumbnail/${encodeURIComponent(path)}?size=360${ownerParam}`;
}

export function personLabel(person) {
  const name = String(person?.name || "").trim();
  return name || t("people.unnamed");
}

export function matchPercent(score) {
  return Math.round(Math.max(0, Math.min(1, Number(score) || 0)) * 100);
}

export function clusterHasMatchSuggestions(cluster) {
  return Array.isArray(cluster?.match_suggestions) && cluster.match_suggestions.length > 0;
}

function bestMatchScore(cluster) {
  const first = cluster?.match_suggestions?.[0];
  return typeof first?.score === "number" ? first.score : 0;
}

export function sortSuggestionClusters(list) {
  return [...list].sort((left, right) => {
    const leftHas = clusterHasMatchSuggestions(left);
    const rightHas = clusterHasMatchSuggestions(right);
    if (leftHas !== rightHas) return rightHas ? 1 : -1;
    if (leftHas && rightHas) {
      const byScore = bestMatchScore(right) - bestMatchScore(left);
      if (byScore !== 0) return byScore;
    }
    return String(left.cluster_id || "").localeCompare(String(right.cluster_id || ""));
  });
}

export function suggestionClusters(suggestions, showSingles) {
  const all = suggestions?.clusters || [];
  const multi = all.filter((cluster) => !cluster.synthetic && (cluster.face_count || cluster.face_ids?.length || 0) > 1);
  if (showSingles) return sortSuggestionClusters(all);
  const singlesWithMatch = all.filter((cluster) => cluster.synthetic && clusterHasMatchSuggestions(cluster));
  const seen = new Set();
  const list = [];
  for (const cluster of [...multi, ...singlesWithMatch]) {
    if (seen.has(cluster.cluster_id)) continue;
    seen.add(cluster.cluster_id);
    list.push(cluster);
  }
  return sortSuggestionClusters(list);
}

export function namedPeople(people) {
  return (people || []).filter((person) => String(person?.name || "").trim().length > 0 && !person.hidden);
}

export function personCenters(person) {
  if (!person) return [];
  const meta = new Map((person.centers || []).map((center) => [center.id, center]));
  const grouped = new Map();
  for (const appearance of person.appearances || []) {
    const centerId = appearance.center_id || "unknown";
    if (!grouped.has(centerId)) grouped.set(centerId, []);
    grouped.get(centerId).push(appearance);
  }
  return [...grouped.entries()].map(([center_id, appearances]) => {
    const info = meta.get(center_id) || {};
    return {
      center_id,
      label: info.label,
      appearance_count: appearances.length,
      appearances,
    };
  });
}

export function centerLabel(center) {
  const label = String(center?.label || "").trim();
  if (label) return label;
  return t("people.clusterFaceCount", { count: center?.appearance_count || 0 });
}

export function detailItemBox(item) {
  return item?.box_xywh || item?.box || null;
}

export function detailItemKey(item) {
  if (item?.face_id) return item.face_id;
  if (item?.identity_key) return item.identity_key;
  return `${item?.source || ""}:${(detailItemBox(item) || []).join(",")}`;
}

function resolveFaceItem(item, catalog) {
  if (!item) return null;
  if (item.source) return item;
  const faceId = item.face_id;
  if (!faceId) return item;
  const meta = catalog?.[faceId];
  if (!meta) return item;
  return { ...item, ...meta };
}

export function clusterFaces(cluster, catalog) {
  if (Array.isArray(cluster?.faces) && cluster.faces.length) {
    return cluster.faces.map((face) => resolveFaceItem(face, catalog)).filter(Boolean);
  }
  return (cluster?.face_ids || [])
    .map((face_id) => resolveFaceItem({ face_id }, catalog))
    .filter(Boolean);
}

export function clusterPreviewFaceIds(cluster, catalog) {
  const faces = clusterFaces(cluster, catalog);
  const fromFaces = faces.slice(0, 16).map((face) => face.face_id).filter(Boolean);
  if (fromFaces.length) return fromFaces;
  const fromIds = (cluster?.face_ids || []).slice(0, 16).filter(Boolean);
  if (fromIds.length) return fromIds;
  const rep = cluster?.representative_face_id;
  return rep ? [rep] : [];
}

function basenameFromPath(path) {
  return String(path || "").split("/").pop() || path || "";
}

export function faceItemToPhoto(item, albumName, albumOwner, catalog) {
  const resolved = resolveFaceItem(item, catalog);
  const source = resolved?.source || "";
  const path = albumPhotoPath(albumName, source);
  if (!path) return null;
  return {
    source,
    path,
    owner: albumOwner || "",
    name: basenameFromPath(path),
    tags: [],
    rating: 0,
    color_label: [],
  };
}

function boxesEqual(left, right) {
  if (!Array.isArray(left) || !Array.isArray(right) || left.length < 4 || right.length < 4) return false;
  return left.every((value, index) => Math.abs(Number(value) - Number(right[index])) < 1e-5);
}

export function isCurrentCover(person, appearance, albumName) {
  if (!person?.cover?.path || !appearance?.source) return false;
  return person.cover.path === albumPhotoPath(albumName, appearance.source)
    && boxesEqual(person.cover.face_box, detailItemBox(appearance));
}
