<script setup>
import { computed, onMounted, ref, watch } from "vue";
import { t } from "../i18n/index.js";
import { formatSize, formatTime } from "../utils/format";
import AppDialog from "./AppDialog.vue";
import StorageDonut from "./StorageDonut.vue";
import { IconDatabase, IconFolder, IconRefresh, IconTrash } from "../utils/icons";

const props = defineProps({
  isDark: { type: Boolean, default: false },
  report: { type: Object, default: null },
  loading: { type: Boolean, default: false },
  error: { type: String, default: "" },
  stale: { type: Boolean, default: false },
  scanning: { type: Boolean, default: false },
  cleaning: { type: Boolean, default: false },
  onRefresh: { type: Function, default: null },
  onScan: { type: Function, default: null },
  onCleanup: { type: Function, default: null },
  onRebuildIndex: { type: Function, default: null },
  /** Rendered inside the settings section: no page hero, tighter spacing. */
  embedded: { type: Boolean, default: false },
  /** Admin can look at other members; everyone else only sees themselves. */
  isAdmin: { type: Boolean, default: false },
  members: { type: Array, default: () => [] },
  membersLoading: { type: Boolean, default: false },
  sharedCache: { type: Array, default: () => [] },
  /** Account whose report is on screen ("" = the caller). */
  viewingOwner: { type: String, default: "" },
  onViewMember: { type: Function, default: null },
  onScanMember: { type: Function, default: null },
  onScanAll: { type: Function, default: null },
  onLoadMembers: { type: Function, default: null },
});

const canCleanup = computed(() => !props.viewingOwner || props.isAdmin);

function memberTotal(row) {
  return row?.totals?.bytes || 0;
}

const MEMBER_COLORS = ["#f59e0b", "#0ea5e9", "#10b981", "#8b5cf6", "#f43f5e", "#14b8a6", "#f97316", "#64748b"];

const memberSegments = computed(() => {
  const rows = props.members
    .filter((row) => row.report_present && memberTotal(row) > 0)
    .map((row, index) => ({
      key: row.owner,
      label: row.owner,
      value: memberTotal(row),
      color: MEMBER_COLORS[index % MEMBER_COLORS.length],
    }));
  const shared = props.sharedCache.reduce((sum, area) => sum + (Number(area.bytes) || 0), 0);
  if (shared > 0) {
    rows.push({
      key: "__shared__",
      label: t("storage.sharedCacheLabel"),
      value: shared,
      color: "#94a3b8",
    });
  }
  return rows;
});

const memberSegmentsTotal = computed(() =>
  memberSegments.value.reduce((sum, segment) => sum + segment.value, 0),
);

function memberShare(row) {
  const total = memberSegmentsTotal.value || props.members.reduce((sum, item) => sum + memberTotal(item), 0);
  return total > 0 ? Math.round((memberTotal(row) / total) * 100) : 0;
}

onMounted(() => {
  if (props.isAdmin) props.onLoadMembers?.();
});

watch(
  () => props.isAdmin,
  (admin) => {
    if (admin) props.onLoadMembers?.();
  },
);

const view = ref("formats");
const confirmSpec = ref(null);
const pending = ref(false);

function kind(name) {
  return (
    props.report?.totals?.[name] || {
      files: 0,
      bytes: 0,
      unattributed_files: 0,
      unattributed_bytes: 0,
    }
  );
}

const thumbBytes = computed(
  () => kind("thumbnails").bytes + kind("darktable_previews").bytes,
);
const originalsBytes = computed(
  () =>
    kind("albums").bytes +
    kind("resources").bytes +
    kind("outside").bytes +
    kind("temp").bytes,
);
const reclaimableBytes = computed(
  () =>
    kind("thumbnails").unattributed_bytes +
    kind("darktable_previews").unattributed_bytes +
    kind("transcodes").unattributed_bytes +
    kind("transcode_probes").unattributed_bytes,
);
const reclaimableFiles = computed(
  () =>
    kind("thumbnails").unattributed_files +
    kind("darktable_previews").unattributed_files +
    kind("transcodes").unattributed_files +
    kind("transcode_probes").unattributed_files,
);

const statCards = computed(() => [
  { label: t("storage.total"), value: formatSize(props.report?.totals?.bytes || 0), note: "" },
  { label: t("storage.originals"), value: formatSize(originalsBytes.value), note: "" },
  {
    label: t("storage.thumbnails"),
    value: formatSize(thumbBytes.value),
    note: t("storage.darktable") + " " + formatSize(kind("darktable_previews").bytes),
  },
  {
    label: t("storage.transcodes"),
    value: formatSize(kind("transcodes").bytes),
    note: t("storage.probes") + " " + formatSize(kind("transcode_probes").bytes),
  },
  {
    label: t("storage.ai"),
    value: formatSize(kind("ai").bytes),
    note: t("storage.aiHint"),
  },
]);

const formats = computed(() => props.report?.formats || []);
const albums = computed(() => props.report?.albums || []);
const months = computed(() => props.report?.months || []);
const videos = computed(() => props.report?.videos || []);
const sizeRows = computed(() => props.report?.thumbnail_sizes || []);
const outsideRows = computed(() => props.report?.outside || []);

const PALETTE = [
  "#f59e0b",
  "#0ea5e9",
  "#10b981",
  "#8b5cf6",
  "#f43f5e",
  "#f97316",
  "#14b8a6",
];
const OTHER_COLOR = "#94a3b8";

const composition = computed(() =>
  [
    { key: "albums", label: t("storage.albums"), value: kind("albums").bytes, color: PALETTE[0] },
    { key: "outside", label: t("storage.outside"), value: kind("outside").bytes, color: PALETTE[1] },
    {
      key: "resources",
      label: t("storage.otherFiles"),
      value: kind("resources").bytes + kind("temp").bytes,
      color: OTHER_COLOR,
    },
    { key: "thumbnails", label: t("storage.thumbnails"), value: thumbBytes.value, color: PALETTE[2] },
    {
      key: "transcodes",
      label: t("storage.transcodes"),
      value: kind("transcodes").bytes + kind("transcode_probes").bytes,
      color: PALETTE[3],
    },
    { key: "ai", label: t("storage.ai"), value: kind("ai").bytes, color: PALETTE[4] },
    {
      key: "cache_other",
      label: t("storage.otherCache"),
      value: kind("cache_other").bytes,
      color: PALETTE[5],
    },
  ].filter((slice) => slice.value > 0),
);

function withRemainder(rows, limit) {
  const slices = rows.slice(0, limit).map((row, index) => ({
    ...row,
    color: PALETTE[index % PALETTE.length],
  }));
  const remainder = rows.slice(limit).reduce((sum, row) => sum + row.value, 0);
  if (remainder > 0) {
    slices.push({ key: "__other__", label: t("storage.other"), value: remainder, color: OTHER_COLOR });
  }
  return slices;
}

const formatSegments = computed(() =>
  withRemainder(
    formats.value.map((row) => ({ key: row.ext, label: row.ext, value: row.bytes })),
    5,
  ),
);

const albumTotalBytes = computed(() =>
  albums.value.reduce(
    (sum, row) => sum + row.bytes + row.thumbnails_bytes + row.transcodes_bytes,
    0,
  ),
);

const albumSegments = computed(() =>
  withRemainder(
    albums.value.map((row) => ({
      key: row.album,
      label: row.name,
      value: row.bytes + row.thumbnails_bytes + row.transcodes_bytes,
    })),
    6,
  ),
);

const maxFormat = computed(() =>
  Math.max(1, ...formats.value.map((row) => row.bytes)),
);
const maxAlbum = computed(() =>
  Math.max(1, ...albums.value.map((row) => row.bytes)),
);
const maxMonth = computed(() =>
  Math.max(1, ...months.value.map((row) => row.bytes)),
);

const yearGroups = computed(() => {
  const groups = new Map();
  for (const row of months.value) {
    const year = row.month.slice(0, 4);
    if (!groups.has(year)) groups.set(year, []);
    groups.get(year).push(row);
  }
  return [...groups.entries()].map(([year, rows]) => ({
    year,
    rows,
    bytes: rows.reduce((sum, row) => sum + row.bytes, 0),
  }));
});

function percent(part, whole) {
  if (!whole) return "0%";
  return `${Math.min(100, Math.round((part / whole) * 100))}%`;
}

function monthLabel(month) {
  return month || t("storage.unknownMonth");
}

function topLabel(top) {
  return top || t("storage.rootFiles");
}

function sizeLabel(size) {
  return size === 0 ? t("storage.fullSize") : t("storage.thumbnailSize", { size });
}

function formatDuration(ms) {
  if (!ms) return "-";
  const total = Math.round(Number(ms) / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = String(total % 60).padStart(2, "0");
  return `${minutes}:${seconds}`;
}

function ask(spec, count, bytes) {
  confirmSpec.value = { spec, count, bytes };
}

function askUnattributedThumbnails() {
  ask(
    { kind: "thumbnails", target: "unattributed" },
    kind("thumbnails").unattributed_files + kind("darktable_previews").unattributed_files,
    kind("thumbnails").unattributed_bytes + kind("darktable_previews").unattributed_bytes,
  );
}

function askAllThumbnails() {
  ask(
    { kind: "thumbnails", target: "all" },
    kind("thumbnails").files + kind("darktable_previews").files,
    kind("thumbnails").bytes + kind("darktable_previews").bytes,
  );
}

function askUnattributedTranscodes() {
  ask(
    { kind: "transcodes", target: "unattributed" },
    kind("transcodes").unattributed_files + kind("transcode_probes").unattributed_files,
    kind("transcodes").unattributed_bytes + kind("transcode_probes").unattributed_bytes,
  );
}

function askAllTranscodes() {
  ask(
    { kind: "transcodes", target: "all" },
    kind("transcodes").files + kind("transcode_probes").files,
    kind("transcodes").bytes + kind("transcode_probes").bytes,
  );
}

function askAlbumThumbnails(album) {
  ask(
    { kind: "thumbnails", target: "album", album: album.album },
    album.thumbnails_files,
    album.thumbnails_bytes,
  );
}

function askAlbumTranscodes(album) {
  ask(
    { kind: "transcodes", target: "album", album: album.album },
    album.transcodes_files,
    album.transcodes_bytes,
  );
}

function askVideo(video) {
  ask(
    { kind: "transcodes", target: "key", key: video.key, codec: video.codec },
    1,
    video.bytes,
  );
}

async function confirmCleanup() {
  if (!confirmSpec.value || pending.value) return;
  pending.value = true;
  const spec = { ...confirmSpec.value.spec, dry_run: false };
  const result = await props.onCleanup?.(spec);
  pending.value = false;
  if (result) confirmSpec.value = null;
}
</script>

<template>
  <section class="app-scrollbar" :class="embedded ? 'overflow-visible' : 'h-full overflow-y-auto px-1 pb-8'">
    <div class="w-full" :class="embedded ? '' : 'mx-auto max-w-[1500px]'">
      <header v-if="!embedded" class="flex flex-wrap items-end justify-between gap-3 py-2 md:py-4">
        <div class="app-dash-island max-w-xl px-4 py-3">
          <p class="app-accent-ink text-xs font-semibold uppercase tracking-[0.18em]">Lumeno</p>
          <h1 class="mt-1 font-display text-2xl font-semibold md:text-3xl" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t("storage.title") }}</h1>
          <p class="mt-1 text-sm" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t("storage.subtitle") }}</p>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <button
            type="button"
            class="app-btn app-btn-primary h-10 px-3.5 text-sm"
            :disabled="scanning"
            @click="onScan?.()"
          >
            <IconRefresh class="h-4 w-4" :class="scanning ? 'animate-spin' : ''" />
            {{ t("storage.rescan") }}
          </button>
          <button type="button" class="app-btn app-btn-secondary h-10 px-3.5 text-sm" @click="onRefresh?.()">
            <IconDatabase class="h-4 w-4" />{{ t("storage.refresh") }}
          </button>
        </div>
      </header>
      <div v-else class="mb-3 flex flex-wrap items-center justify-between gap-2">
        <p class="text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("storage.subtitle") }}</p>
        <div class="flex items-center gap-2">
          <button
            type="button"
            class="app-btn app-btn-primary h-8 px-2.5 text-xs"
            :disabled="scanning"
            @click="onScan?.()"
          >
            <IconRefresh class="h-3.5 w-3.5" :class="scanning ? 'animate-spin' : ''" />
            {{ t("storage.rescan") }}
          </button>
          <button type="button" class="app-btn app-btn-secondary h-8 px-2.5 text-xs" @click="onRefresh?.()">
            <IconDatabase class="h-3.5 w-3.5" />{{ t("storage.refresh") }}
          </button>
        </div>
      </div>

      <section v-if="isAdmin" class="app-dash-island mb-3 px-4 py-3">
        <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
          <div class="min-w-0">
            <p class="text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-700'">{{ t("storage.members") }}</p>
            <p class="mt-0.5 text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-500'">{{ t("storage.membersHint") }}</p>
          </div>
          <div class="flex items-center gap-2">
            <button type="button" class="app-btn app-btn-secondary h-8 px-2.5 text-xs" :disabled="membersLoading" @click="onScanAll?.()">
              <IconRefresh class="h-3.5 w-3.5" :class="membersLoading ? 'animate-spin' : ''" />
              {{ t("storage.scanAllMembers") }}
            </button>
          </div>
        </div>
        <div class="grid gap-4 xl:grid-cols-[minmax(0,1fr)_16rem]">
        <div class="min-w-0 overflow-x-auto">
          <div class="min-w-[42rem]">
            <div class="grid grid-cols-[minmax(6rem,1fr)_6.5rem_5rem_5rem_4.5rem_6.5rem_7rem] items-center gap-2 border-b pb-1 text-[10px] font-medium uppercase tracking-wide"
              :class="isDark ? 'border-stone-800 text-stone-500' : 'border-stone-200 text-stone-500'">
              <span>{{ t("storage.member") }}</span>
              <span class="text-right">{{ t("storage.memberTotal") }}</span>
              <span class="text-right">{{ t("storage.originals") }}</span>
              <span class="text-right">{{ t("storage.thumbnails") }}</span>
              <span class="text-right">{{ t("storage.ai") }}</span>
              <span class="text-right">{{ t("storage.memberScannedAt") }}</span>
              <span class="text-right">{{ t("storage.memberActions") }}</span>
            </div>
            <div v-for="row in members" :key="row.owner"
              class="grid grid-cols-[minmax(6rem,1fr)_6.5rem_5rem_5rem_4.5rem_6.5rem_7rem] items-center gap-2 border-b py-1.5 text-xs last:border-b-0"
              :class="isDark ? 'border-stone-800/60' : 'border-stone-100'">
              <span class="flex min-w-0 items-center gap-1.5">
                <span class="truncate font-medium" :class="viewingOwner === row.owner ? 'app-accent-ink' : (isDark ? 'text-stone-100' : 'text-stone-800')">{{ row.owner }}</span>
                <span v-if="row.is_self" class="shrink-0 rounded bg-stone-500/15 px-1 py-0.5 text-[10px]" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t("storage.memberSelf") }}</span>
                <span v-if="row.scanning" class="shrink-0 text-[10px] text-amber-500">{{ t("storage.scanning") }}</span>
              </span>
              <span class="text-right tabular-nums" :class="isDark ? 'text-stone-200' : 'text-stone-800'">
                {{ row.report_present ? formatSize(memberTotal(row)) : "—" }}
                <span v-if="row.report_present && row.totals.bytes" class="ml-1 text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-500'">{{ memberShare(row) }}%</span>
              </span>
              <span class="text-right tabular-nums" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ row.report_present ? formatSize(row.totals.originals_bytes) : "—" }}</span>
              <span class="text-right tabular-nums" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ row.report_present ? formatSize(row.totals.thumbnails_bytes + row.totals.darktable_bytes) : "—" }}</span>
              <span class="text-right tabular-nums" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ row.report_present ? formatSize(row.totals.ai_bytes) : "—" }}</span>
              <span class="text-right tabular-nums" :class="isDark ? 'text-stone-500' : 'text-stone-500'">{{ row.generated_at ? formatTime(row.generated_at) : t("storage.memberNever") }}</span>
              <span class="flex items-center justify-end gap-1">
                <button type="button" class="rounded-md px-1.5 py-0.5 text-[11px]"
                  :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
                  :disabled="row.scanning"
                  @click="onScanMember?.(row.owner)">{{ t("storage.memberScan") }}</button>
                <button type="button" class="rounded-md px-1.5 py-0.5 text-[11px]"
                  :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
                  :disabled="!row.report_present"
                  @click="onViewMember?.(row.owner)">{{ t("storage.memberView") }}</button>
              </span>
            </div>
            <p v-if="!members.length" class="py-3 text-center text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-500'">{{ t("storage.memberEmpty") }}</p>
          </div>
        </div>
        <div v-if="memberSegments.length" class="min-w-0 xl:pl-2">
          <p class="mb-1 text-[11px] font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("storage.memberChart") }}</p>
          <StorageDonut
            :segments="memberSegments"
            :center-label="t('storage.memberChartCenter')"
            :center-value="formatSize(memberSegmentsTotal)"
            :format="formatSize"
            :is-dark="isDark"
            :size="132"
          />
        </div>
        </div>
        <p v-if="sharedCache.length" class="mt-2 text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-500'">
          {{ t("storage.sharedCacheHint") }}
          <span v-for="(area, index) in sharedCache" :key="area.name">
            <span v-if="index" class="mx-1">·</span>{{ area.name }} {{ formatSize(area.bytes) }}
          </span>
        </p>
      </section>

      <div v-if="viewingOwner" class="app-dash-island mb-3 flex flex-wrap items-center justify-between gap-2 border-l-4 border-sky-500 px-4 py-2.5 text-xs">
        <span :class="isDark ? 'text-stone-200' : 'text-stone-700'">
          {{ isAdmin
            ? t("storage.memberViewAdminCleanup", { owner: viewingOwner })
            : t("storage.memberViewOnly", { owner: viewingOwner }) }}
        </span>
        <button type="button" class="app-btn app-btn-secondary h-7 px-2 text-xs" @click="onViewMember?.('')">{{ t("storage.backToSelf") }}</button>
      </div>

      <div v-if="error" class="app-dash-island my-3 border-l-4 border-rose-500 px-4 py-3 text-sm text-rose-800">
        {{ error }}
      </div>
      <div v-if="scanning" class="app-dash-island my-3 border-l-4 border-amber-500 px-4 py-3 text-sm" :class="isDark ? 'text-amber-200' : 'text-amber-800'">
        {{ t("storage.scanning") }}
      </div>
      <div v-else-if="cleaning" class="app-dash-island my-3 border-l-4 border-amber-500 px-4 py-3 text-sm" :class="isDark ? 'text-amber-200' : 'text-amber-800'">
        {{ t("storage.cleaning") }}
      </div>
      <div v-else-if="stale && report" class="app-dash-island my-3 border-l-4 border-amber-500 px-4 py-3 text-sm" :class="isDark ? 'text-amber-200' : 'text-amber-800'">
        {{ t("storage.staleHint") }}
      </div>

      <div v-if="loading && !report" class="flex min-h-80 items-center justify-center">
        <div class="h-8 w-8 animate-spin rounded-full border-2 border-amber-500 border-t-transparent"></div>
      </div>

      <div v-else-if="!report" class="app-dash-island my-4 px-6 py-10 text-center">
        <p class="text-sm" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t("storage.noReport") }}</p>
        <button type="button" class="app-btn app-btn-primary mt-4 h-10 px-4 text-sm" @click="onScan?.()">
          {{ t("storage.startScan") }}
        </button>
      </div>

      <template v-else>
        <section class="flex flex-col gap-3">
          <div class="grid grid-cols-2 gap-2.5 md:grid-cols-5">
            <div v-for="card in statCards" :key="card.label" class="app-dash-island min-w-0 px-3 py-3">
              <p class="flex items-center gap-1.5 truncate text-[11px] font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                <IconDatabase class="app-accent h-3 w-3 shrink-0" />
                {{ card.label }}
              </p>
              <p class="mt-1 truncate text-lg font-semibold tabular-nums" :class="isDark ? 'text-stone-50' : 'text-stone-900'">{{ card.value }}</p>
              <p v-if="card.note" class="mt-0.5 truncate text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-500'">{{ card.note }}</p>
            </div>
          </div>

          <div class="app-dash-island min-w-0 px-4 py-3">
            <p class="mb-1 text-[11px] font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("storage.composition") }}</p>
            <StorageDonut
              row
              :segments="composition"
              :center-label="t('storage.total')"
              :center-value="formatSize(report.totals?.bytes || 0)"
              :format="formatSize"
              :is-dark="isDark"
              :size="132"
            />
          </div>

          <p class="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-500'">
            <span>{{ t("storage.reclaimable", { size: formatSize(reclaimableBytes) }) }}</span>
            <span v-if="reclaimableFiles">· {{ t("storage.unattributedCount", { count: reclaimableFiles }) }}</span>
            <span>· {{ t("storage.generatedAt", { time: formatTime(report.generated_at) }) }}</span>
            <span v-if="report.shared_album_count">· {{ t("storage.sharedHint", { count: report.shared_album_count }) }}</span>
          </p>
        </section>

        <div class="mt-5 inline-flex rounded-xl border p-1" :class="isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white'">
          <button
            v-for="tab in ['formats', 'albums', 'months']"
            :key="tab"
            type="button"
            class="rounded-lg px-3 py-1.5 text-xs font-medium transition"
            :class="view === tab
              ? 'bg-amber-600 text-white'
              : isDark ? 'text-stone-300 hover:bg-stone-800' : 'text-stone-600 hover:bg-stone-100'"
            @click="view = tab"
          >
            {{ t(tab === 'formats' ? 'storage.byFormat' : tab === 'albums' ? 'storage.byAlbum' : 'storage.byMonth') }}
          </button>
        </div>

        <section v-if="view === 'formats'" class="app-dash-island mt-3 px-4 py-4">
          <div class="grid gap-5 lg:grid-cols-[minmax(15rem,19rem)_minmax(0,1fr)]">
            <div class="min-w-0">
              <StorageDonut
                :segments="formatSegments"
                :center-label="t('storage.originals')"
                :center-value="formatSize(originalsBytes)"
                :format="formatSize"
                :is-dark="isDark"
                :size="150"
              />
            </div>
            <div class="min-w-0">
              <div
                class="mb-2 grid grid-cols-[4.5rem_4rem_1fr_6.5rem] items-center gap-2 text-[10px] font-medium uppercase tracking-wide"
                :class="isDark ? 'text-stone-500' : 'text-stone-500'"
              >
                <span>{{ t("storage.ext") }}</span>
                <span class="text-right">{{ t("storage.files") }}</span>
                <span />
                <span class="text-right">{{ t("storage.size") }}</span>
              </div>
              <div
                v-for="row in formats"
                :key="row.ext"
                class="grid grid-cols-[4.5rem_4rem_1fr_6.5rem] items-center gap-2 py-1.5 text-sm"
              >
                <span class="truncate font-medium" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ row.ext }}</span>
                <span class="text-right tabular-nums" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ row.files }}</span>
                <span class="h-2 overflow-hidden rounded-full" :class="isDark ? 'bg-white/10' : 'bg-stone-200'">
                  <span class="block h-full rounded-full bg-amber-500" :style="{ width: percent(row.bytes, maxFormat) }" />
                </span>
                <span class="text-right tabular-nums" :class="isDark ? 'text-stone-200' : 'text-stone-700'">
                  {{ formatSize(row.bytes) }}
                  <span v-if="row.cache_bytes" class="ml-1 whitespace-nowrap text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-500'">
                    +{{ formatSize(row.cache_bytes) }}
                  </span>
                </span>
              </div>
              <p v-if="!formats.length" class="py-6 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("storage.empty") }}</p>
            </div>
          </div>
        </section>

        <section v-else-if="view === 'albums'" class="app-dash-island mt-3 px-4 py-4">
          <div class="grid gap-5 xl:grid-cols-[minmax(0,1fr)_19rem]">
            <div class="min-w-0">
              <div
                class="grid grid-cols-[minmax(0,1fr)_4rem_6rem_6rem_6rem_7rem] items-center gap-2 border-b pb-2 text-[11px] font-medium uppercase tracking-wide"
                :class="isDark ? 'border-stone-800 text-stone-500' : 'border-stone-200 text-stone-500'"
              >
                <span>{{ t("storage.album") }}</span>
                <span class="text-right">{{ t("storage.files") }}</span>
                <span class="text-right">{{ t("storage.originals") }}</span>
                <span class="text-right">{{ t("storage.thumbnails") }}</span>
                <span class="text-right">{{ t("storage.transcodes") }}</span>
                <span class="text-right">{{ t("storage.albumTotal") }}</span>
              </div>
              <div
                v-for="row in albums"
                :key="row.album"
                class="grid grid-cols-[minmax(0,1fr)_4rem_6rem_6rem_6rem_7rem] items-center gap-2 py-1.5 text-sm"
              >
                <span class="flex min-w-0 items-center gap-1.5">
                  <span class="truncate font-medium" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ row.name }}</span>
                  <button
                    v-if="!row.indexed"
                    type="button"
                    class="shrink-0 rounded-md px-1.5 py-0.5 text-[10px]"
                    :class="isDark ? 'bg-stone-800 text-stone-300' : 'bg-stone-200 text-stone-600'"
                    :title="t('storage.rebuildIndex')"
                    @click="onRebuildIndex?.(row.album)"
                  >{{ t("storage.rebuildIndex") }}</button>
                </span>
                <span class="text-right tabular-nums" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ row.files }}</span>
                <span class="text-right tabular-nums" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ formatSize(row.bytes) }}</span>
                <span class="flex items-center justify-end gap-1 tabular-nums" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
                  {{ formatSize(row.thumbnails_bytes) }}
                  <button
                    v-if="canCleanup && row.thumbnails_bytes"
                    type="button"
                    class="rounded-md p-1"
                    :class="isDark ? 'hover:bg-stone-800' : 'hover:bg-stone-100'"
                    :title="t('storage.cleanAlbumThumbnails')"
                    @click="askAlbumThumbnails(row)"
                  >
                    <IconTrash class="h-3.5 w-3.5" />
                  </button>
                </span>
                <span class="flex items-center justify-end gap-1 tabular-nums" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
                  {{ formatSize(row.transcodes_bytes) }}
                  <button
                    v-if="canCleanup && row.transcodes_bytes"
                    type="button"
                    class="rounded-md p-1"
                    :class="isDark ? 'hover:bg-stone-800' : 'hover:bg-stone-100'"
                    :title="t('storage.cleanAlbumTranscodes')"
                    @click="askAlbumTranscodes(row)"
                  >
                    <IconTrash class="h-3.5 w-3.5" />
                  </button>
                </span>
                <span class="text-right font-medium tabular-nums" :class="isDark ? 'text-stone-100' : 'text-stone-900'">
                  {{ formatSize(row.bytes + row.thumbnails_bytes + row.transcodes_bytes) }}
                </span>
              </div>
              <p v-if="!albums.length" class="py-6 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("storage.empty") }}</p>
            </div>
            <div class="min-w-0 xl:pl-2">
              <p class="mb-2 text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t("storage.albumShare") }}</p>
              <StorageDonut
                :segments="albumSegments"
                :center-label="t('storage.albums')"
                :center-value="formatSize(albumTotalBytes)"
                :format="formatSize"
                :is-dark="isDark"
                :size="150"
              />
            </div>
          </div>
        </section>

        <section v-else class="app-dash-island mt-3 px-4 py-4">
          <div v-for="group in yearGroups" :key="group.year" class="mb-4 last:mb-0">
            <p class="mb-1 text-xs font-semibold" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
              {{ group.year }} · {{ formatSize(group.bytes) }}
            </p>
            <div v-for="row in group.rows" :key="row.month" class="grid grid-cols-[5.5rem_4rem_1fr_7rem] items-center gap-2 py-1 text-sm">
              <span class="tabular-nums" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ row.month }}</span>
              <span class="text-right tabular-nums" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ row.files }}</span>
              <span class="flex h-2 overflow-hidden rounded-full" :class="isDark ? 'bg-white/10' : 'bg-stone-200'">
                <span class="block h-full bg-amber-500" :style="{ width: percent(row.bytes, maxMonth) }" />
                <span class="block h-full bg-sky-500/70" :style="{ width: percent(row.thumbnails_bytes + row.transcodes_bytes, maxMonth) }" />
              </span>
              <span class="text-right tabular-nums" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ formatSize(row.bytes) }}</span>
            </div>
          </div>
          <p v-if="!months.length" class="py-6 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("storage.empty") }}</p>
          <p v-if="report.unindexed_albums" class="mt-2 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
            {{ t("storage.unindexedHint", { count: report.unindexed_albums }) }}
          </p>
        </section>

        <section v-if="outsideRows.length" class="app-dash-island mt-3 px-4 py-4">
          <p class="flex items-center gap-1.5 text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
            <IconFolder class="app-accent h-4 w-4" />{{ t("storage.outsideTitle") }}
          </p>
          <p class="mt-1 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("storage.outsideHint") }}</p>
          <div v-for="row in outsideRows" :key="row.top" class="mt-2 flex items-center justify-between gap-3 text-sm">
            <span class="truncate" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ topLabel(row.top) }}</span>
            <span class="shrink-0 tabular-nums" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ row.files }} · {{ formatSize(row.bytes) }}</span>
          </div>
        </section>

        <section class="app-dash-island mt-3 px-4 py-4">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <p class="text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ t("storage.thumbnailCache") }}</p>
            <div v-if="canCleanup" class="flex flex-wrap gap-2">
              <button
                type="button"
                class="rounded-xl px-3 py-1.5 text-xs font-medium"
                :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
                :disabled="!(
                  kind('thumbnails').unattributed_files +
                  kind('darktable_previews').unattributed_files
                )"
                @click="askUnattributedThumbnails"
              >{{ t("storage.cleanUnattributed") }}</button>
              <button
                type="button"
                class="rounded-xl px-3 py-1.5 text-xs font-medium"
                :class="isDark ? 'bg-rose-900/60 text-rose-200 hover:bg-rose-800' : 'bg-rose-100 text-rose-700 hover:bg-rose-200'"
                :disabled="!thumbBytes"
                @click="askAllThumbnails"
              >{{ t("storage.cleanAllThumbnails") }}</button>
            </div>
          </div>
          <p class="mt-1 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
            {{ kind("thumbnails").files }} · {{ formatSize(kind("thumbnails").bytes) }}
            <span v-if="kind('darktable_previews').files">· {{ t("storage.darktable") }} {{ kind("darktable_previews").files }} · {{ formatSize(kind("darktable_previews").bytes) }}</span>
          </p>
          <div class="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
            <span v-for="row in sizeRows.slice(0, 8)" :key="row.size">
              {{ sizeLabel(row.size) }} · {{ row.files }} · {{ formatSize(row.bytes) }}
            </span>
          </div>
          <p v-if="kind('thumbnails').unattributed_files || kind('darktable_previews').unattributed_files" class="mt-2 text-xs" :class="isDark ? 'text-amber-300' : 'text-amber-700'">
            {{ t("storage.unattributed") }}
            {{ kind("thumbnails").unattributed_files + kind("darktable_previews").unattributed_files }}
            · {{ formatSize(kind("thumbnails").unattributed_bytes + kind("darktable_previews").unattributed_bytes) }}
            <span class="opacity-80">— {{ t("storage.unattributedHint") }}</span>
          </p>
        </section>

        <section class="app-dash-island mt-3 px-4 py-4">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <p class="text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ t("storage.transcodeCache") }}</p>
            <div v-if="canCleanup" class="flex flex-wrap gap-2">
              <button
                type="button"
                class="rounded-xl px-3 py-1.5 text-xs font-medium"
                :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
                :disabled="!kind('transcodes').unattributed_files"
                @click="askUnattributedTranscodes"
              >{{ t("storage.cleanUnattributed") }}</button>
              <button
                type="button"
                class="rounded-xl px-3 py-1.5 text-xs font-medium"
                :class="isDark ? 'bg-rose-900/60 text-rose-200 hover:bg-rose-800' : 'bg-rose-100 text-rose-700 hover:bg-rose-200'"
                :disabled="!kind('transcodes').files"
                @click="askAllTranscodes"
              >{{ t("storage.cleanAllTranscodes") }}</button>
            </div>
          </div>
          <p class="mt-1 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
            {{ kind("transcodes").files }} · {{ formatSize(kind("transcodes").bytes) }}
          </p>
          <div class="mt-2 max-h-72 overflow-y-auto app-scrollbar">
            <div v-for="video in videos" :key="video.codec + video.key" class="flex items-center justify-between gap-3 border-b py-1.5 text-sm last:border-0" :class="isDark ? 'border-stone-800' : 'border-stone-100'">
              <span class="min-w-0 flex-1 truncate" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ video.name }}</span>
              <span class="shrink-0 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                <span v-if="video.album">{{ video.album }} · </span>
                <span v-if="video.unattributed" class="text-amber-600">{{ t("storage.unattributed") }} · </span>
                {{ video.codec.toUpperCase() }} · {{ formatDuration(video.duration_ms) }} · {{ formatSize(video.bytes) }}
              </span>
              <button
                v-if="canCleanup"
                type="button"
                class="shrink-0 rounded-lg p-1.5"
                :class="isDark ? 'text-stone-300 hover:bg-stone-800' : 'text-stone-600 hover:bg-stone-100'"
                :title="t('storage.deleteOne')"
                @click="askVideo(video)"
              >
                <IconTrash class="h-4 w-4" />
              </button>
            </div>
          </div>
          <p v-if="!videos.length" class="py-4 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("storage.noVideos") }}</p>
        </section>

        <p class="mt-4 text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-500'">{{ t("storage.aiHint") }} · {{ t("storage.tasksHint") }}</p>
      </template>
    </div>

    <AppDialog :show="Boolean(confirmSpec)" :z-index="120" @close="confirmSpec = null">
      <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-md rounded-2xl border p-4" :class="isDark ? 'text-stone-100' : 'text-stone-900'">
        <h3 class="mb-2 text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ t("storage.confirmTitle") }}</h3>
        <p class="mb-4 text-xs leading-relaxed" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
          {{ viewingOwner
            ? t("storage.confirmBodyMember", { owner: viewingOwner, count: confirmSpec?.count || 0, size: formatSize(confirmSpec?.bytes || 0) })
            : t("storage.confirmBody", { count: confirmSpec?.count || 0, size: formatSize(confirmSpec?.bytes || 0) }) }}
        </p>
        <div class="flex justify-end gap-2">
          <button type="button" class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="confirmSpec = null">
            {{ t("storage.cancel") }}
          </button>
          <button type="button" class="inline-flex items-center gap-1.5 rounded-xl bg-rose-600 px-3 py-1.5 text-xs font-medium text-white disabled:opacity-70" :disabled="pending" @click="confirmCleanup">
            <span v-if="pending" class="h-3 w-3 animate-spin rounded-full border-2 border-white/70 border-t-transparent" />
            {{ pending ? t("storage.cleaning") : (viewingOwner
              ? t("storage.confirmDeleteMember", { owner: viewingOwner })
              : t("storage.confirmDelete")) }}
          </button>
        </div>
      </div>
    </AppDialog>
  </section>
</template>
