<script setup>
import { computed, ref, watch } from "vue";
import {
  IconClose, IconDatabase, IconWand, IconSearch, IconImage, IconCamera, IconRefresh,
  IconPencil, IconCheck,
} from "../utils/icons";
import { t } from "../i18n/index.js";
import FaceAvatar from "./FaceAvatar.vue";
import { personDisplayCover, parseAlbumPhotoPath, personPhotoCount } from "../utils/faceCrop.js";
import AppDialog from "./AppDialog.vue";
import AppSlider from "./AppSlider.vue";

const props = defineProps({
  isDark: { type: Boolean, default: false },
  isMobile: { type: Boolean, default: false },
  albumViewMode: { type: String, default: "photos" },
  initialQuery: { type: String, default: "" },
  similarLoading: { type: Boolean, default: false },
  people: { type: Array, default: () => [] },
  peopleLoading: { type: Boolean, default: false },
  albumName: { type: String, default: "" },
  albumOwner: { type: String, default: "" },
  selectedPerson: { type: Object, default: null },
  personPhotos: { type: Array, default: () => [] },
  personPhotosLoading: { type: Boolean, default: false },
});

const emit = defineEmits([
  "close",
  "search-text",
  "show-duplicates",
  "show-photos",
  "filter-person",
  "refresh-people",
  "select-person",
  "back-to-people",
  "patch-person",
  "merge-people",
  "split-person",
]);

const query = ref(props.initialQuery || "");
const minScore = ref(0);
const limit = ref(60);
const showUnnamed = ref(false);
const renameOpen = ref(false);
const renameValue = ref("");
const mergeOpen = ref(false);
const mergeTargetId = ref("");
const splitOpen = ref(false);
const splitNewName = ref("");
const selectedFaceIds = ref([]);

watch(
  () => props.initialQuery,
  (value) => {
    if (typeof value === "string" && value !== query.value) {
      query.value = value;
    }
  },
);

watch(
  () => props.selectedPerson,
  (person) => {
    renameValue.value = person?.name || "";
    selectedFaceIds.value = [];
    splitNewName.value = "";
  },
);

const inSpecialView = computed(() =>
  ["duplicates", "similar", "text"].includes(props.albumViewMode),
);

/** Only named (user-designated) faces by default. */
const visiblePeople = computed(() => {
  const list = Array.isArray(props.people) ? props.people : [];
  return list.filter((person) => {
    if (person?.hidden) return false;
    if (showUnnamed.value) return true;
    return String(person?.name || "").trim().length > 0;
  });
});

const canEditSelected = computed(() => Boolean(props.selectedPerson?.editable));

const selectableFaces = computed(() => {
  const faces = [];
  for (const photo of props.personPhotos || []) {
    const ids = photo.face_ids || [];
    const boxes = photo.face_boxes || [];
    ids.forEach((faceId, index) => {
      faces.push({
        faceId,
        path: photo.path,
        owner: photo.owner,
        source: parseAlbumPhotoPath(photo.path)?.source || "",
        box: boxes[index] || null,
      });
    });
  }
  return faces;
});

function runTextSearch() {
  const text = String(query.value || "").trim();
  if (!text) return;
  emit("search-text", {
    text,
    scope: "current_album",
    minScore: Number(minScore.value) || 0,
    limit: Math.max(1, Math.min(200, Number(limit.value) || 60)),
  });
}

function personLabel(person) {
  const name = String(person?.name || "").trim();
  return name || t("people.unnamed");
}

function personAvatarCover(person) {
  return personDisplayCover(person, props.albumName, props.albumOwner);
}

function openManage(person) {
  emit("select-person", person);
}

function backToFaces() {
  emit("back-to-people");
}

async function saveRename() {
  if (!props.selectedPerson) return;
  emit("patch-person", props.selectedPerson.id, { name: renameValue.value.trim() });
  renameOpen.value = false;
}

async function toggleHidden() {
  if (!props.selectedPerson) return;
  emit("patch-person", props.selectedPerson.id, { hidden: !props.selectedPerson.hidden });
}

function confirmMerge() {
  if (!props.selectedPerson || !mergeTargetId.value) return;
  emit("merge-people", {
    target_person_id: mergeTargetId.value,
    source_person_ids: [props.selectedPerson.id],
  });
  mergeOpen.value = false;
}

function toggleFaceSelection(faceId) {
  if (selectedFaceIds.value.includes(faceId)) {
    selectedFaceIds.value = selectedFaceIds.value.filter((id) => id !== faceId);
  } else {
    selectedFaceIds.value = [...selectedFaceIds.value, faceId];
  }
}

function confirmSplit() {
  if (!props.selectedPerson || selectedFaceIds.value.length === 0) return;
  emit("split-person", {
    source_person_id: props.selectedPerson.id,
    face_ids: [...selectedFaceIds.value],
    new_name: splitNewName.value.trim(),
  });
  splitOpen.value = false;
  selectedFaceIds.value = [];
  splitNewName.value = "";
}
</script>

<template>
  <aside
    class="app-scrollbar app-frost flex h-full shrink-0 flex-col overflow-y-auto border-l"
    :class="[
      isDark ? 'border-stone-700/50' : 'border-white/40',
      isMobile ? 'w-full' : 'w-72',
    ]"
  >
    <div class="flex items-center justify-between gap-2 border-b px-3 py-2.5" :class="isDark ? 'border-stone-700/60' : 'border-stone-200/60'">
      <div class="flex min-w-0 items-center gap-1.5">
        <IconWand class="h-4 w-4 shrink-0 text-amber-500" />
        <p class="truncate text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
          {{ t("albums.aiTools") }}
        </p>
      </div>
      <button
        class="rounded-full p-1.5 transition"
        :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
        @click="emit('close')"
      >
        <IconClose class="h-3.5 w-3.5" />
      </button>
    </div>

    <div class="flex flex-col gap-4 p-3">
      <!-- Text search -->
      <section class="flex flex-col gap-2">
        <p class="text-[10px] font-bold uppercase tracking-wider" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
          {{ t("albums.aiTextSearch") }}
        </p>
        <textarea
          v-model="query"
          rows="3"
          :placeholder="t('albums.textSearchPlaceholder')"
          class="w-full resize-none rounded-xl border px-2.5 py-2 text-xs"
          :class="isDark ? 'border-stone-700 bg-stone-950 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'"
          @keydown.enter.exact.prevent="runTextSearch"
        />
        <label class="flex flex-col gap-1">
          <span class="flex items-center justify-between text-[11px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
            <span>{{ t("albums.aiMinScore") }}</span>
            <span class="tabular-nums">{{ Number(minScore).toFixed(2) }}</span>
          </span>
          <AppSlider class="w-full shrink-0 grow-0" v-model="minScore" :min="0" :max="1" :step="0.01" :is-dark="isDark" />
        </label>
        <span class="text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t("albums.aiMinScoreHint") }}</span>
        <label class="flex flex-col gap-1">
          <span class="text-[11px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("albums.aiResultLimit") }}</span>
          <input
            v-model.number="limit"
            type="number"
            min="1"
            max="200"
            class="rounded-lg border px-2 py-1.5 text-xs"
            :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
          />
        </label>
        <button
          class="inline-flex items-center justify-center gap-1.5 rounded-xl px-3 py-2 text-xs font-medium transition"
          :class="isDark ? 'bg-amber-600 text-white hover:bg-amber-500' : 'bg-amber-600 text-white hover:bg-amber-700'"
          :disabled="similarLoading || !String(query || '').trim()"
          @click="runTextSearch"
        >
          <IconSearch class="h-3.5 w-3.5" />
          {{ similarLoading ? t("albums.aiSearching") : t("albums.aiRunSearch") }}
        </button>
      </section>

      <!-- Actions -->
      <section class="flex flex-col gap-2">
        <p class="text-[10px] font-bold uppercase tracking-wider" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
          {{ t("albums.aiActions") }}
        </p>
        <button
          class="inline-flex items-center gap-1.5 rounded-xl px-3 py-2 text-left text-xs font-medium transition"
          :class="albumViewMode === 'duplicates'
            ? (isDark ? 'bg-amber-500/20 text-amber-200' : 'bg-amber-100 text-amber-900')
            : (isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300')"
          @click="emit('show-duplicates')"
        >
          <IconDatabase class="h-3.5 w-3.5 shrink-0" />
          <span class="min-w-0 flex-1">{{ t("albums.showDuplicates") }}</span>
        </button>
        <p class="text-[10px] leading-relaxed" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
          {{ t("albums.aiSimilarHint") }}
        </p>
        <button
          v-if="inSpecialView"
          class="inline-flex items-center gap-1.5 rounded-xl px-3 py-2 text-left text-xs font-medium transition"
          :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
          @click="emit('show-photos')"
        >
          <IconImage class="h-3.5 w-3.5 shrink-0" />
          {{ t("albums.backToPhotos") }}
        </button>
      </section>

      <!-- Faces management -->
      <section class="flex flex-col gap-2">
        <div class="flex items-center justify-between gap-2">
          <p class="flex items-center gap-1 text-[10px] font-bold uppercase tracking-wider" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
            <IconCamera class="h-3 w-3" />
            {{ selectedPerson ? personLabel(selectedPerson) : t("albums.aiFaces") }}
          </p>
          <div class="flex items-center gap-1">
            <button
              v-if="selectedPerson"
              class="rounded px-1.5 py-1 text-[10px] font-medium transition"
              :class="isDark ? 'text-stone-400 hover:bg-stone-800' : 'text-stone-500 hover:bg-stone-200'"
              @click="backToFaces"
            >
              {{ t("people.back") }}
            </button>
            <button
              class="rounded p-1 transition"
              :class="isDark ? 'text-stone-500 hover:bg-stone-800 hover:text-stone-300' : 'text-stone-400 hover:bg-stone-200 hover:text-stone-600'"
              :title="t('people.refresh')"
              @click="emit('refresh-people')"
            >
              <IconRefresh class="h-3 w-3" />
            </button>
          </div>
        </div>

        <template v-if="selectedPerson">
          <div v-if="canEditSelected" class="flex flex-wrap gap-1.5">
            <button
              class="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-[10px] font-medium transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
              @click="renameOpen = true"
            >
              <IconPencil class="h-3 w-3" />{{ t("people.rename") }}
            </button>
            <button
              class="rounded-lg px-2 py-1 text-[10px] font-medium transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
              @click="toggleHidden"
            >
              {{ selectedPerson.hidden ? t("people.unhide") : t("people.hide") }}
            </button>
            <button
              class="rounded-lg px-2 py-1 text-[10px] font-medium transition"
              :class="isDark ? 'bg-amber-500/20 text-amber-200' : 'bg-amber-100 text-amber-900'"
              @click="mergeOpen = true"
            >
              {{ t("people.merge") }}
            </button>
            <button
              class="rounded-lg px-2 py-1 text-[10px] font-medium transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
              @click="splitOpen = true"
            >
              {{ t("people.split") }}
            </button>
          </div>
          <p class="text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
            {{ t("people.photoCount", { count: personPhotoCount(selectedPerson) || personPhotos.length }) }}
          </p>
          <div v-if="personPhotosLoading" class="py-3 text-center text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
            {{ t("people.loadingPhotos") }}
          </div>
        </template>

        <template v-else>
          <p class="text-[10px] leading-relaxed" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
            {{ t("albums.aiFacesHint") }}
          </p>
          <label class="inline-flex items-center gap-2 text-[11px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
            <input v-model="showUnnamed" type="checkbox" class="rounded accent-amber-500" />
            {{ t("albums.aiShowUnnamedFaces") }}
          </label>
          <div v-if="peopleLoading" class="py-3 text-center text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
            {{ t("people.loading") }}
          </div>
          <div v-else-if="visiblePeople.length === 0" class="rounded-xl border px-3 py-4 text-center text-[11px]" :class="isDark ? 'border-stone-800 text-stone-500' : 'border-stone-200 text-stone-400'">
            {{ showUnnamed ? t("albums.aiFacesEmpty") : t("albums.aiNamedFacesEmpty") }}
          </div>
          <div v-else class="grid grid-cols-3 gap-2">
            <div
              v-for="person in visiblePeople"
              :key="person.id"
              class="flex flex-col items-center gap-1 rounded-xl p-1.5 text-center"
              :class="isDark ? 'hover:bg-stone-800' : 'hover:bg-stone-100'"
            >
              <button type="button" class="h-12 w-12 overflow-hidden rounded-full" :class="isDark ? 'bg-stone-800' : 'bg-stone-200'" @click="emit('filter-person', person)">
                <FaceAvatar
                  v-if="personAvatarCover(person)"
                  :album="albumName"
                  :cover="personAvatarCover(person)"
                  :size="48"
                />
              </button>
              <button type="button" class="w-full truncate text-[10px] font-medium" :class="isDark ? 'text-stone-200' : 'text-stone-700'" @click="emit('filter-person', person)">
                {{ personLabel(person) }}
              </button>
              <button
                type="button"
                class="text-[9px] font-medium underline-offset-2 hover:underline"
                :class="isDark ? 'text-amber-300/80' : 'text-amber-700'"
                @click="openManage(person)"
              >
                {{ t("albums.aiManageFace") }}
              </button>
            </div>
          </div>
        </template>
      </section>
    </div>

    <AppDialog :show="renameOpen" :z-index="120" @close="renameOpen = false">
        <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-md rounded-2xl border p-4" :class="isDark ? 'text-stone-100' : 'text-stone-900'">
          <h3 class="mb-3 text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ t("people.rename") }}</h3>
          <input
            v-model="renameValue"
            type="text"
            class="mb-4 w-full rounded-lg border px-3 py-2 text-sm"
            :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
            :placeholder="t('people.namePlaceholder')"
          />
          <div class="flex justify-end gap-2">
            <button class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="renameOpen = false">{{ t("albums.cancel") }}</button>
            <button class="rounded-xl px-3 py-1.5 text-xs font-medium bg-amber-600 text-white" @click="saveRename">{{ t("albums.save") }}</button>
          </div>
        </div>
    </AppDialog>

    <AppDialog :show="mergeOpen" :z-index="120" @close="mergeOpen = false">
        <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-md rounded-2xl border p-4" :class="isDark ? 'text-stone-100' : 'text-stone-900'">
          <h3 class="mb-3 text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ t("people.merge") }}</h3>
          <p class="mb-3 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("people.mergeHint") }}</p>
          <select
            v-model="mergeTargetId"
            class="mb-4 w-full rounded-lg border px-3 py-2 text-sm"
            :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
          >
            <option value="">{{ t("people.mergeSelectTarget") }}</option>
            <option v-for="person in visiblePeople.filter((p) => p.id !== selectedPerson?.id)" :key="person.id" :value="person.id">
              {{ personLabel(person) }}
            </option>
          </select>
          <div class="flex justify-end gap-2">
            <button class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="mergeOpen = false">{{ t("albums.cancel") }}</button>
            <button class="rounded-xl px-3 py-1.5 text-xs font-medium bg-amber-600 text-white" :disabled="!mergeTargetId" @click="confirmMerge">{{ t("people.merge") }}</button>
          </div>
        </div>
    </AppDialog>

    <AppDialog :show="splitOpen" :z-index="120" @close="splitOpen = false">
        <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-lg rounded-2xl border p-4" :class="isDark ? 'text-stone-100' : 'text-stone-900'">
          <h3 class="mb-3 text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ t("people.split") }}</h3>
          <p class="mb-3 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("people.splitHint") }}</p>
          <div v-if="selectableFaces.length === 0" class="mb-4 text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
            {{ t("people.splitNoFaces") }}
          </div>
          <div v-else class="mb-4 grid max-h-56 grid-cols-3 gap-2 overflow-y-auto">
            <button
              v-for="face in selectableFaces"
              :key="face.faceId"
              type="button"
              class="relative overflow-hidden rounded-xl border p-1 text-left transition"
              :class="selectedFaceIds.includes(face.faceId)
                ? (isDark ? 'border-amber-400 ring-1 ring-amber-400/60' : 'border-amber-500 ring-1 ring-amber-500/50')
                : (isDark ? 'border-stone-700' : 'border-stone-200')"
              @click="toggleFaceSelection(face.faceId)"
            >
              <div class="relative aspect-square overflow-hidden rounded-lg" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                <FaceAvatar
                  :album="albumName"
                  :face-id="face.faceId"
                  :source="face.source"
                  :face-box="face.box"
                  :size="80"
                />
              </div>
              <span
                v-if="selectedFaceIds.includes(face.faceId)"
                class="absolute right-1.5 top-1.5 flex h-5 w-5 items-center justify-center rounded-full bg-amber-500 text-white"
              >
                <IconCheck class="h-3 w-3" />
              </span>
            </button>
          </div>
          <input
            v-model="splitNewName"
            type="text"
            class="mb-4 w-full rounded-lg border px-3 py-2 text-sm"
            :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
            :placeholder="t('people.namePlaceholder')"
          />
          <div class="flex justify-end gap-2">
            <button class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="splitOpen = false">{{ t("albums.cancel") }}</button>
            <button
              class="rounded-xl px-3 py-1.5 text-xs font-medium bg-amber-600 text-white disabled:opacity-50"
              :disabled="selectedFaceIds.length === 0"
              @click="confirmSplit"
            >{{ t("people.split") }}</button>
          </div>
        </div>
    </AppDialog>
  </aside>
</template>
