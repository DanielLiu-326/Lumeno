<script setup>
import { computed, onBeforeUnmount, ref, watch } from "vue";
import AppDialog from "../../components/AppDialog.vue";
import FaceAvatar from "../../components/FaceAvatar.vue";
import { t } from "../../i18n/index.js";
import { personDisplayCover, personPhotoCount } from "../../utils/faceCrop.js";
import { IconPencil, IconTrash } from "../../utils/icons";
import ActionSheet from "../sheets/ActionSheet.vue";
import {
  clusterHasMatchSuggestions,
  clusterPreviewFaceIds,
  matchPercent,
  namedPeople,
  personLabel,
  suggestionClusters,
} from "./faceModel.js";

const props = defineProps({
  isDark: { type: Boolean, default: false },
  albumName: { type: String, default: "" },
  albumOwner: { type: String, default: "" },
  busy: { type: Boolean, default: false },
  hintsLoading: { type: Boolean, default: false },
  people: { type: Array, default: () => [] },
  suggestions: { type: Object, default: () => ({ clusters: [] }) },
  editable: { type: Boolean, default: true },
});

const emit = defineEmits(["open-person", "open-cluster", "merge-cluster", "delete-person", "patch-person"]);

const showSingles = ref(false);
const revealed = ref(false);
const menuPerson = ref(null);
const deleteTarget = ref(null);
const renamePerson = ref(null);
const renameValue = ref("");
const menuActions = computed(() => [
  { id: "rename", label: t("people.rename"), icon: IconPencil },
  { id: "delete", label: t("people.delete"), icon: IconTrash, danger: true },
]);
const peopleList = computed(() => namedPeople(props.people));
const clusters = computed(() => suggestionClusters(props.suggestions, showSingles.value));
const loading = computed(() => !revealed.value && (props.busy || props.hintsLoading));

watch(
  () => props.busy || props.hintsLoading,
  (pending) => {
    if (!pending) revealed.value = true;
  },
  { immediate: true },
);

const catalog = computed(() => props.suggestions?.face_catalog || {});

function coverOf(person) {
  return personDisplayCover(person, props.albumName, props.albumOwner);
}

function onMenu(action) {
  const person = menuPerson.value;
  menuPerson.value = null;
  if (!person || !action?.id) return;
  if (action.id === "rename") {
    renamePerson.value = person;
    renameValue.value = person.name || "";
    return;
  }
  if (action.id === "delete") deleteTarget.value = person;
}

function saveRename() {
  const person = renamePerson.value;
  const name = renameValue.value.trim();
  if (!person?.id || !name) return;
  emit("patch-person", person.id, { name });
  renamePerson.value = null;
}

function confirmDelete() {
  const person = deleteTarget.value;
  if (!person?.id) return;
  emit("delete-person", person.id);
  deleteTarget.value = null;
}

let press = null;
let timer = 0;
let swallowClick = false;

function clearPress() {
  window.removeEventListener("pointermove", onPersonMove);
  window.removeEventListener("pointerup", onPersonUp);
  window.removeEventListener("pointercancel", onPersonCancel);
  press = null;
}

function cancelTimer() {
  if (!timer) return;
  clearTimeout(timer);
  timer = 0;
}

function onPersonDown(person, event) {
  if (!props.editable) return;
  if (event.button != null && event.button !== 0) return;
  clearPress();
  cancelTimer();
  press = { person, x: event.clientX, y: event.clientY, moved: false };
  timer = window.setTimeout(() => {
    timer = 0;
    if (!press || press.moved) return;
    menuPerson.value = press.person;
    swallowClick = true;
    window.setTimeout(() => {
      swallowClick = false;
    }, 700);
  }, 450);
  window.addEventListener("pointermove", onPersonMove);
  window.addEventListener("pointerup", onPersonUp);
  window.addEventListener("pointercancel", onPersonCancel);
}

function onPersonMove(event) {
  if (!press || press.moved) return;
  if (Math.abs(event.clientX - press.x) > 12 || Math.abs(event.clientY - press.y) > 12) {
    press.moved = true;
    cancelTimer();
  }
}

function onPersonUp() {
  clearPress();
  cancelTimer();
}

function onPersonCancel() {
  clearPress();
  cancelTimer();
}

function onPersonClick(person) {
  if (swallowClick) {
    swallowClick = false;
    return;
  }
  emit("open-person", person);
}

onBeforeUnmount(() => {
  clearPress();
  cancelTimer();
});
</script>

<template>
  <div class="app-scrollbar min-h-0 flex-1 overflow-y-auto px-3 py-3">
    <div v-if="loading" class="flex flex-col items-center justify-center gap-3 py-16">
      <div class="h-8 w-8 animate-spin rounded-full border-2 border-amber-500 border-t-transparent" />
      <p class="text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("people.loading") }}</p>
    </div>
    <template v-else>
      <h3 class="mb-2 text-xs font-bold uppercase tracking-wider" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        {{ t("people.title") }}
        <span class="font-normal normal-case">({{ peopleList.length }})</span>
      </h3>
      <div v-if="peopleList.length === 0" class="mb-5 rounded-xl border px-3 py-4 text-center text-[11px]" :class="isDark ? 'border-stone-800 text-stone-500' : 'border-stone-200 text-stone-400'">
        {{ t("albums.aiNamedFacesEmpty") }}
      </div>
      <div v-else class="mb-6 grid grid-cols-3 gap-2">
        <button
          v-for="person in peopleList"
          :key="person.id"
          type="button"
          class="relative flex w-full flex-col items-center gap-1 rounded-xl p-2 text-center"
          :class="isDark ? 'bg-stone-800/50' : 'bg-stone-50'"
          @pointerdown="onPersonDown(person, $event)"
          @click="onPersonClick(person)"
          @contextmenu.prevent
        >
            <div class="h-14 w-14 overflow-hidden rounded-full" :class="isDark ? 'bg-stone-700' : 'bg-stone-200'">
              <FaceAvatar v-if="coverOf(person)" :album="albumName" :cover="coverOf(person)" :size="56" />
            </div>
            <p class="w-full truncate text-[11px] font-medium" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ personLabel(person) }}</p>
            <p class="text-[9px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t("people.photoCount", { count: personPhotoCount(person) }) }}</p>
        </button>
      </div>

      <div class="mb-2 flex items-center justify-between gap-2">
        <h3 class="text-xs font-bold uppercase tracking-wider" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
          {{ t("albums.aiSuggestions") }}
          <span class="font-normal normal-case">({{ clusters.length }})</span>
        </h3>
        <label class="inline-flex items-center gap-1.5 text-[10px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
          <input v-model="showSingles" type="checkbox" class="rounded accent-amber-500" />
          {{ t("albums.aiShowSingles") }}
        </label>
      </div>
      <div v-if="clusters.length === 0" class="rounded-xl border px-3 py-4 text-center text-[11px]" :class="isDark ? 'border-stone-800 text-stone-500' : 'border-stone-200 text-stone-400'">
        {{ t("albums.aiFacesEmpty") }}
      </div>
      <div v-else class="flex flex-col gap-2">
        <button
          v-for="cluster in clusters"
          :key="cluster.cluster_id"
          type="button"
          class="rounded-xl border p-3 text-left"
          :class="isDark ? 'border-stone-700 bg-stone-950/40' : 'border-stone-200 bg-white'"
          @click="emit('open-cluster', cluster)"
        >
          <p class="text-xs font-semibold" :class="isDark ? 'text-stone-200' : 'text-stone-800'">
            {{ cluster.synthetic ? t("albums.aiSingleFace") : t("albums.aiCluster") }}
            · {{ t("people.photoCount", { count: cluster.face_count || cluster.face_ids?.length || 0 }) }}
          </p>
          <div v-if="clusterHasMatchSuggestions(cluster)" class="mt-2 flex flex-wrap gap-1.5" @click.stop>
            <button
              v-for="suggestion in cluster.match_suggestions"
              :key="`${suggestion.person_id}:${suggestion.center_id || ''}`"
              type="button"
              class="rounded-full px-2 py-0.5 text-[10px] font-medium"
              :class="isDark ? 'bg-sky-900/50 text-sky-100' : 'bg-sky-100 text-sky-900'"
              @click="emit('merge-cluster', { target_person_id: suggestion.person_id, cluster_id: cluster.cluster_id, center_label: 'variant' })"
            >
              {{ t("albums.aiMatchSuggest", { name: suggestion.person_name, percent: matchPercent(suggestion.score) }) }}
            </button>
          </div>
          <div class="mt-2 flex flex-wrap gap-1.5">
            <div
              v-for="faceId in clusterPreviewFaceIds(cluster, catalog)"
              :key="faceId"
              class="h-11 w-11 overflow-hidden rounded-full"
              :class="isDark ? 'bg-stone-800' : 'bg-stone-200'"
            >
              <FaceAvatar :album="albumName" :face-id="faceId" :size="44" />
            </div>
          </div>
        </button>
      </div>
    </template>
    <ActionSheet
      :show="!!menuPerson"
      :title="menuPerson ? personLabel(menuPerson) : ''"
      :actions="menuActions"
      @close="menuPerson = null"
      @select="onMenu"
    />
    <AppDialog :show="!!renamePerson" :z-index="130" @close="renamePerson = null">
      <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-md rounded-2xl border p-4" :class="isDark ? 'text-stone-100' : 'text-stone-900'">
        <h3 class="mb-3 text-sm font-semibold">{{ t("people.rename") }}</h3>
        <input
          v-model="renameValue"
          type="text"
          class="mb-4 w-full rounded-lg border px-3 py-2 text-sm"
          :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
          :placeholder="t('people.namePlaceholder')"
          @keyup.enter="saveRename"
        />
        <div class="flex justify-end gap-2">
          <button type="button" class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="renamePerson = null">{{ t("albums.cancel") }}</button>
          <button type="button" class="rounded-xl bg-amber-600 px-3 py-1.5 text-xs font-medium text-white" @click="saveRename">{{ t("albums.save") }}</button>
        </div>
      </div>
    </AppDialog>
    <AppDialog :show="!!deleteTarget" :z-index="130" @close="deleteTarget = null">
      <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-md rounded-2xl border p-4" :class="isDark ? 'text-stone-100' : 'text-stone-900'">
        <h3 class="mb-2 text-sm font-semibold">{{ t("people.delete") }}</h3>
        <p class="mb-4 text-xs leading-relaxed" :class="isDark ? 'text-stone-400' : 'text-stone-600'">
          {{ t("people.deleteConfirm", { name: personLabel(deleteTarget) }) }}
        </p>
        <div class="flex justify-end gap-2">
          <button type="button" class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="deleteTarget = null">{{ t("albums.cancel") }}</button>
          <button type="button" class="rounded-xl bg-red-600 px-3 py-1.5 text-xs font-medium text-white" @click="confirmDelete">{{ t("people.delete") }}</button>
        </div>
      </div>
    </AppDialog>
  </div>
</template>
