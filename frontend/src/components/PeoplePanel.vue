<script setup>
import { ref, computed, watch } from "vue";
import {
  IconArrowLeft, IconCamera, IconCheck, IconClose, IconPencil, IconSparkles,
} from "../utils/icons";
import { t } from "../i18n/index.js";
import LazyImage from "./LazyImage.vue";
import FaceAvatar from "./FaceAvatar.vue";

const props = defineProps({
  isDark: { type: Boolean, default: false },
  busy: { type: Boolean, default: false },
  people: { type: Array, default: () => [] },
  peopleTotal: { type: Number, default: 0 },
  selectedPerson: { type: Object, default: null },
  personPhotos: { type: Array, default: () => [] },
  personPhotosLoading: { type: Boolean, default: false },
  onRefresh: { type: Function, default: null },
  onSelectPerson: { type: Function, default: null },
  onBackToPeople: { type: Function, default: null },
  onPatchPerson: { type: Function, default: null },
  onMergePeople: { type: Function, default: null },
  onSplitPerson: { type: Function, default: null },
  formatSize: { type: Function, default: (n) => String(n) },
});

const showHidden = ref(false);
const renameOpen = ref(false);
const renameValue = ref("");
const mergeOpen = ref(false);
const mergeTargetId = ref("");
const splitOpen = ref(false);
const splitNewName = ref("");
const selectedFaceIds = ref([]);

const visiblePeople = computed(() =>
  props.people.filter((person) => showHidden.value || !person.hidden),
);

const canEditSelected = computed(() => Boolean(props.selectedPerson?.editable));

const selectableFaces = computed(() => {
  const faces = [];
  for (const photo of props.personPhotos) {
    const ids = photo.face_ids || [];
    const boxes = photo.face_boxes || [];
    ids.forEach((faceId, index) => {
      faces.push({
        faceId,
        path: photo.path,
        owner: photo.owner,
        box: boxes[index] || null,
        label: photo.name || String(photo.path || "").split("/").pop() || faceId,
      });
    });
  }
  return faces;
});

watch(
  () => props.selectedPerson,
  (person) => {
    renameValue.value = person?.name || "";
    selectedFaceIds.value = [];
    splitNewName.value = "";
  },
  { immediate: true },
);

function personLabel(person) {
  const name = (person?.name || "").trim();
  return name || t("people.unnamed");
}

function toggleFaceSelection(faceId) {
  if (selectedFaceIds.value.includes(faceId)) {
    selectedFaceIds.value = selectedFaceIds.value.filter((id) => id !== faceId);
  } else {
    selectedFaceIds.value = [...selectedFaceIds.value, faceId];
  }
}

function openPerson(person) {
  props.onSelectPerson?.(person);
}

function backToGrid() {
  props.onBackToPeople?.();
}

async function saveRename() {
  if (!props.selectedPerson || !props.onPatchPerson) return;
  await props.onPatchPerson(props.selectedPerson.id, { name: renameValue.value.trim() });
  renameOpen.value = false;
}

async function toggleHidden() {
  if (!props.selectedPerson || !props.onPatchPerson) return;
  await props.onPatchPerson(props.selectedPerson.id, { hidden: !props.selectedPerson.hidden });
}

async function confirmMerge() {
  if (!props.selectedPerson || !mergeTargetId.value || !props.onMergePeople) return;
  await props.onMergePeople({
    target_person_id: mergeTargetId.value,
    source_person_ids: [props.selectedPerson.id],
  });
  mergeOpen.value = false;
}

async function confirmSplit() {
  if (!props.selectedPerson || !props.onSplitPerson || selectedFaceIds.value.length === 0) return;
  await props.onSplitPerson({
    source_person_id: props.selectedPerson.id,
    face_ids: [...selectedFaceIds.value],
    new_name: splitNewName.value.trim(),
  });
  splitOpen.value = false;
  selectedFaceIds.value = [];
  splitNewName.value = "";
}

function photoThumb(path, owner) {
  const ownerParam = owner ? `&owner=${encodeURIComponent(owner)}` : "";
  return `/api/thumbnail/${encodeURIComponent(path)}?size=360${ownerParam}`;
}
</script>

<template>
  <div class="flex h-full min-h-0 flex-col">
    <div class="flex items-center gap-2 border-b px-3 py-3 md:px-4" :class="isDark ? 'border-stone-700/60' : 'border-stone-200/60'">
      <button
        v-if="selectedPerson"
        class="inline-flex items-center gap-1 rounded-xl px-2.5 py-1.5 text-xs font-medium transition"
        :class="isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
        @click="backToGrid"
      >
        <IconArrowLeft class="h-3.5 w-3.5" />{{ t("people.back") }}
      </button>
      <h2 class="text-sm font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
        {{ selectedPerson ? personLabel(selectedPerson) : t("people.title") }}
      </h2>
      <div class="flex-1"></div>
      <label v-if="!selectedPerson" class="inline-flex items-center gap-2 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        <input v-model="showHidden" type="checkbox" class="rounded accent-amber-500" />
        {{ t("people.showHidden") }}
      </label>
      <button
        v-if="!selectedPerson && onRefresh"
        class="inline-flex items-center gap-1 rounded-xl px-2.5 py-1.5 text-xs font-medium transition"
        :class="isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
        @click="onRefresh()"
      >
        <IconSparkles class="h-3.5 w-3.5" />{{ t("people.refresh") }}
      </button>
    </div>

    <!-- Person detail -->
    <div v-if="selectedPerson" class="app-scrollbar min-h-0 flex-1 overflow-y-auto p-3 md:p-4">
      <div v-if="canEditSelected" class="mb-4 flex flex-wrap items-center gap-2">
        <button
          class="inline-flex items-center gap-1 rounded-xl px-3 py-1.5 text-xs font-medium transition"
          :class="isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
          @click="renameOpen = true"
        >
          <IconPencil class="h-3.5 w-3.5" />{{ t("people.rename") }}
        </button>
        <button
          class="inline-flex items-center gap-1 rounded-xl px-3 py-1.5 text-xs font-medium transition"
          :class="isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
          @click="toggleHidden"
        >
          {{ selectedPerson.hidden ? t("people.unhide") : t("people.hide") }}
        </button>
        <button
          class="inline-flex items-center gap-1 rounded-xl px-3 py-1.5 text-xs font-medium transition"
          :class="isDark ? 'bg-amber-500/20 text-amber-200' : 'bg-amber-100 text-amber-900'"
          @click="mergeOpen = true"
        >
          {{ t("people.merge") }}
        </button>
        <button
          class="inline-flex items-center gap-1 rounded-xl px-3 py-1.5 text-xs font-medium transition"
          :class="isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
          @click="splitOpen = true"
        >
          {{ t("people.split") }}
        </button>
      </div>
      <p
        v-else
        class="mb-4 text-xs"
        :class="isDark ? 'text-stone-400' : 'text-stone-500'"
      >
        {{ t("people.readOnlyShared") }}
      </p>

      <p class="mb-3 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        {{ t("people.photoCount", { count: selectedPerson.visible_photo_count ?? personPhotos.length }) }}
      </p>

      <div v-if="personPhotosLoading" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        {{ t("people.loadingPhotos") }}
      </div>
      <div v-else-if="personPhotos.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        {{ t("people.noPhotos") }}
      </div>
      <div v-else class="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
        <article
          v-for="photo in personPhotos"
          :key="photo.path || photo.stable_id"
          class="rounded-xl border p-1.5 sm:p-2"
          :class="isDark ? 'border-stone-700 bg-stone-900/80' : 'border-stone-200 bg-white/80'"
        >
          <div class="relative aspect-square overflow-hidden rounded-lg" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
            <LazyImage
              img-class="h-full w-full object-cover"
              :src="photoThumb(photo.path, photo.owner)"
              alt=""
            />
          </div>
          <p class="mt-1.5 truncate text-[11px] font-medium" :class="isDark ? 'text-stone-200' : 'text-stone-700'">
            {{ photo.name || photo.path?.split("/").pop() }}
          </p>
        </article>
      </div>
    </div>

    <!-- People grid -->
    <div v-else class="app-scrollbar min-h-0 flex-1 overflow-y-auto p-3 md:p-4">
      <div v-if="busy && visiblePeople.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        {{ t("people.loading") }}
      </div>
      <div v-else-if="!busy && visiblePeople.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        {{ t("people.empty") }}
      </div>
      <div v-else class="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
        <article
          v-for="person in visiblePeople"
          :key="person.id"
          class="group cursor-pointer rounded-2xl border p-3 transition hover:shadow-md"
          :class="isDark ? 'border-stone-700 bg-stone-900/70 hover:border-stone-600' : 'border-stone-200 bg-white/80 hover:border-stone-300'"
          @click="openPerson(person)"
        >
          <div class="relative mx-auto mb-2 aspect-square w-full max-w-[8rem] overflow-hidden rounded-full" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
            <FaceAvatar
              v-if="person.cover?.path"
              :cover="person.cover"
              :size="128"
              img-class="transition group-hover:scale-105"
            />
            <div v-else class="flex h-full w-full items-center justify-center" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
              <IconCamera class="h-8 w-8" />
            </div>
          </div>
          <p class="truncate text-center text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
            {{ personLabel(person) }}
          </p>
          <p class="text-center text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
            {{ t("people.photoCount", { count: person.visible_photo_count ?? 0 }) }}
          </p>
        </article>
      </div>
    </div>

    <!-- Rename dialog -->
    <teleport to="body">
      <div v-if="renameOpen" class="fixed inset-0 z-[120] flex items-center justify-center bg-black/50 p-4" @click.self="renameOpen = false">
        <div class="w-full max-w-md rounded-2xl border p-4 shadow-xl" :class="isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white'">
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
            <button class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="isDark ? 'bg-amber-600 text-white' : 'bg-amber-600 text-white'" @click="saveRename">{{ t("albums.save") }}</button>
          </div>
        </div>
      </div>
    </teleport>

    <!-- Merge dialog (simplified) -->
    <teleport to="body">
      <div v-if="mergeOpen" class="fixed inset-0 z-[120] flex items-center justify-center bg-black/50 p-4" @click.self="mergeOpen = false">
        <div class="w-full max-w-md rounded-2xl border p-4 shadow-xl" :class="isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white'">
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
      </div>
    </teleport>

    <!-- Split dialog -->
    <teleport to="body">
      <div v-if="splitOpen" class="fixed inset-0 z-[120] flex items-center justify-center bg-black/50 p-4" @click.self="splitOpen = false">
        <div class="w-full max-w-lg rounded-2xl border p-4 shadow-xl" :class="isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white'">
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
                  :path="face.path"
                  :owner="face.owner"
                  :face-box="face.box"
                  :size="100"
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
      </div>
    </teleport>
  </div>
</template>
