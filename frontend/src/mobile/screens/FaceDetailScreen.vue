<script setup>
import { computed, inject, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import AppDialog from "../../components/AppDialog.vue";
import FaceAvatar from "../../components/FaceAvatar.vue";
import { t } from "../../i18n/index.js";
import { IconArrowLeft, IconDots, IconPencil, IconTrash } from "../../utils/icons";
import { ALBUM_APP_KEY, MOBILE_UI_KEY } from "../context.js";
import FacePhotoGrid from "../faces/FacePhotoGrid.vue";
import {
  centerLabel,
  clusterFaces,
  detailItemBox,
  detailItemKey,
  faceItemToPhoto,
  matchPercent,
  namedPeople,
  personCenters,
  personLabel,
} from "../faces/faceModel.js";
import { albumFaceCenterLocation, albumLocation } from "../router.js";
import Screen from "../layouts/Screen.vue";
import ActionSheet from "../sheets/ActionSheet.vue";

defineOptions({ name: "FaceDetailScreen" });

const app = inject(ALBUM_APP_KEY);
const ui = inject(MOBILE_UI_KEY);
const route = useRoute();
const router = useRouter();

const showOriginal = ref(false);
const showCenters = ref(false);
const menuOpen = ref(false);
const mergeOpen = ref(false);
const nameOpen = ref(false);
const nameValue = ref("");
const deleteOpen = ref(false);
const removeTarget = ref(null);
const sawPerson = ref(false);
const sawCluster = ref(false);

function queryValue(key) {
  const value = route.query[key];
  return Array.isArray(value) ? String(value[0] || "") : String(value || "");
}

const dirName = computed(() => {
  const raw = route.params.dirName;
  if (Array.isArray(raw)) return raw.filter(Boolean).join("/");
  return String(raw || "");
});

const personId = computed(() => queryValue("person"));
const clusterId = computed(() => queryValue("cluster"));
const onFaceRoute = computed(() => route.name === "album-face" || route.name === "album-face-center");

const person = computed(() =>
  (app.albumPeople || []).find((item) => item.id === personId.value) || null,
);

const cluster = computed(() =>
  (app.albumFaceSuggestionsDisplay?.clusters || []).find((item) => item.cluster_id === clusterId.value) || null,
);

const catalog = computed(() => app.albumFaceSuggestionsDisplay?.face_catalog || {});
const albumName = computed(() => app.currentAlbum || dirName.value);
const albumOwner = computed(() => app.albumFacesOwner || app.currentUser || "");
const editable = computed(() => !!app.albumFacesEditable);
const centers = computed(() => personCenters(person.value));
const hasMultipleCenters = computed(() => (person.value?.center_count || person.value?.centers?.length || centers.value.length) > 1);
const clusterItems = computed(() => clusterFaces(cluster.value, catalog.value));
const people = computed(() => namedPeople(app.albumPeople));

const title = computed(() => {
  if (personId.value) return personLabel(person.value);
  if (cluster.value?.synthetic) return t("albums.aiSingleFace");
  return t("albums.aiCluster");
});

const photoCount = computed(() => {
  if (person.value) return person.value.appearance_count ?? (person.value.appearances || []).length;
  return cluster.value?.face_count || clusterItems.value.length || 0;
});

const menuActions = computed(() => {
  if (person.value) {
    return [
      { id: "rename", label: t("people.rename"), icon: IconPencil, disabled: !editable.value },
      { id: "delete", label: t("people.delete"), icon: IconTrash, danger: true, disabled: !editable.value },
    ];
  }
  const actions = [
    { id: "name", label: t("albums.aiNameFace"), icon: IconPencil, disabled: !editable.value },
  ];
  for (const suggestion of cluster.value?.match_suggestions || []) {
    actions.push({
      id: `match:${suggestion.person_id}`,
      label: t("albums.aiMatchMerge", { name: suggestion.person_name, percent: matchPercent(suggestion.score) }),
      disabled: !editable.value,
    });
  }
  if (people.value.length) {
    actions.push({ id: "merge", label: t("people.merge"), disabled: !editable.value });
  }
  return actions;
});

const mergeActions = computed(() =>
  people.value.map((item) => ({ id: item.id, label: personLabel(item) })),
);

watch(
  () => [route.name, dirName.value, personId.value, clusterId.value],
  async ([name, dir, personKey, clusterKey]) => {
    if (name === "album-face-center") return;
    if (name !== "album-face") return;
    if (!dir || (!personKey && !clusterKey)) {
      router.replace(dir ? albumLocation(dir) : { name: "albums" });
      return;
    }
    if (app.currentAlbum !== dir) await app.selectAlbum(dir);
  },
  { immediate: true },
);

watch(person, (next) => {
  if (!personId.value || route.name === "album-face-center") return;
  if (next) {
    sawPerson.value = true;
    return;
  }
  if (sawPerson.value && route.name === "album-face") goBack();
});

watch(cluster, (next) => {
  if (!clusterId.value || personId.value) return;
  if (next) {
    sawCluster.value = true;
    return;
  }
  if (sawCluster.value && route.name === "album-face") goBack();
});

function goBack() {
  ui.markPushClose?.();
  if (window.history.length > 1) {
    router.back();
    return;
  }
  router.replace(albumLocation(dirName.value));
}

function openPhoto(item, items) {
  const navPhotos = (items || []).map((entry) => faceItemToPhoto(entry, albumName.value, albumOwner.value, catalog.value)).filter(Boolean);
  const photo = faceItemToPhoto(item, albumName.value, albumOwner.value, catalog.value);
  if (!photo) return;
  ui.albumPanelRef?.openAiToolsPhoto?.({ photo, navPhotos });
}

function setCover(appearance) {
  if (!person.value?.id || !appearance?.source) return;
  app.patchAlbumPerson(person.value.id, {
    cover: { source: appearance.source, box_xywh: detailItemBox(appearance) },
  });
}

function clearCover() {
  if (!person.value?.id) return;
  app.patchAlbumPerson(person.value.id, { cover: null });
}

function openCenter(center) {
  if (!personId.value || !center?.center_id) return;
  router.push(albumFaceCenterLocation(dirName.value, { person: personId.value, center: center.center_id }));
}

function onMenu(action) {
  menuOpen.value = false;
  if (!action?.id || action.disabled) return;
  if (action.id === "rename") {
    nameValue.value = person.value?.name || "";
    nameOpen.value = true;
    return;
  }
  if (action.id === "name") {
    nameValue.value = "";
    nameOpen.value = true;
    return;
  }
  if (action.id === "delete") {
    deleteOpen.value = true;
    return;
  }
  if (action.id === "merge") {
    mergeOpen.value = true;
    return;
  }
  if (String(action.id).startsWith("match:")) {
    mergeCluster(String(action.id).slice(6));
  }
}

function saveName() {
  const name = nameValue.value.trim();
  if (!name) return;
  if (person.value?.id) {
    app.patchAlbumPerson(person.value.id, { name });
    nameOpen.value = false;
    return;
  }
  if (!cluster.value?.cluster_id) return;
  app.adoptAlbumCluster?.({ cluster_id: cluster.value.cluster_id, name });
  nameOpen.value = false;
}

function confirmDelete() {
  if (!person.value?.id) return;
  deleteOpen.value = false;
  app.deleteAlbumPerson(person.value.id);
}

function mergeCluster(targetId) {
  if (!cluster.value?.cluster_id || !targetId) return;
  mergeOpen.value = false;
  app.mergeAlbumCluster?.({
    target_person_id: targetId,
    cluster_id: cluster.value.cluster_id,
    center_label: "variant",
  });
}

function askRemoveCenter(center) {
  if (!editable.value || !person.value?.id || !center?.center_id) return;
  removeTarget.value = center;
}

function confirmRemoveCenter() {
  const center = removeTarget.value;
  if (!person.value?.id || !center?.center_id) return;
  removeTarget.value = null;
  app.removeAlbumPersonCenter?.({ person_id: person.value.id, center_id: center.center_id });
}
</script>

<template>
  <Screen v-show="onFaceRoute" class="h-full">
    <template #bar>
      <header class="mobile-topbar">
        <button type="button" class="mobile-icon-btn" @click="goBack">
          <IconArrowLeft class="h-5 w-5" />
        </button>
        <h1 class="min-w-0 flex-1 truncate text-base font-semibold">{{ title }}</h1>
        <button type="button" class="mobile-icon-btn" @click="menuOpen = true">
          <IconDots class="h-5 w-5" />
        </button>
      </header>
    </template>

    <div v-if="person" class="flex min-h-0 flex-1 flex-col">
      <p class="shrink-0 px-3 pt-3 text-[11px]" :class="ui.isDark ? 'text-stone-500' : 'text-stone-400'">
        {{ t("people.photoCount", { count: photoCount }) }}
        <template v-if="hasMultipleCenters">
          · {{ person.center_count || person.centers?.length || centers.length }} {{ t("people.naturalClusterCount") }}
        </template>
      </p>
      <div v-if="showCenters && hasMultipleCenters" class="app-scrollbar min-h-0 flex-1 overflow-y-auto px-3 py-3">
        <div class="flex flex-col gap-2">
          <article
            v-for="center in centers"
            :key="center.center_id"
            class="relative cursor-pointer rounded-xl border p-3"
            :class="ui.isDark ? 'border-stone-700 bg-stone-950/40' : 'border-stone-200 bg-white'"
            @click="openCenter(center)"
          >
            <button
              v-if="editable"
              type="button"
              class="absolute right-2 top-2 rounded-md p-1"
              :class="ui.isDark ? 'text-red-300' : 'text-red-700'"
              :title="t('people.removeCluster')"
              @click.stop="askRemoveCenter(center)"
            >
              <IconTrash class="h-4 w-4" />
            </button>
            <p class="mb-2 pr-8 text-xs font-semibold" :class="ui.isDark ? 'text-stone-200' : 'text-stone-800'">
              {{ centerLabel(center) }}
            </p>
            <div class="flex flex-wrap gap-1.5">
              <button
                v-for="appearance in (center.appearances || []).slice(0, 16)"
                :key="detailItemKey(appearance)"
                type="button"
                class="h-11 w-11 overflow-hidden rounded-full"
                :class="ui.isDark ? 'bg-stone-800' : 'bg-stone-200'"
                @click.stop="openPhoto(appearance, center.appearances || [])"
              >
                <FaceAvatar
                  v-if="appearance.source && detailItemBox(appearance)"
                  :album="albumName"
                  :face-id="appearance.face_id || ''"
                  :source="appearance.source || ''"
                  :face-box="detailItemBox(appearance)"
                  :size="44"
                />
              </button>
            </div>
          </article>
        </div>
      </div>
      <FacePhotoGrid
        v-else
        :items="person.appearances || []"
        :is-dark="ui.isDark"
        :album-name="albumName"
        :album-owner="albumOwner"
        :person="person"
        :editable="editable"
        :show-original="showOriginal"
        @open="openPhoto($event, person.appearances || [])"
        @set-cover="setCover"
        @clear-cover="clearCover"
      />
    </div>

    <div v-else-if="cluster" class="flex min-h-0 flex-1 flex-col">
      <p class="shrink-0 px-3 pt-3 text-[11px]" :class="ui.isDark ? 'text-stone-500' : 'text-stone-400'">
        {{ t("people.photoCount", { count: photoCount }) }}
      </p>
      <div
        v-if="clusterItems.length === 0"
        class="mx-3 mt-3 rounded-xl border px-3 py-6 text-center text-[11px]"
        :class="ui.isDark ? 'border-stone-800 text-stone-500' : 'border-stone-200 text-stone-400'"
      >
        {{ t("albums.aiFacePreviewEmpty") }}
      </div>
      <FacePhotoGrid
        v-else
        :items="clusterItems"
        :is-dark="ui.isDark"
        :album-name="albumName"
        :album-owner="albumOwner"
        :show-original="showOriginal"
        @open="openPhoto($event, clusterItems)"
      />
    </div>

    <ActionSheet :show="menuOpen" :title="title" @close="menuOpen = false" @select="onMenu">
      <template #default="{ pick }">
        <label class="mobile-sheet-action">
          <input v-model="showOriginal" type="checkbox" class="h-4 w-4 accent-amber-500" />
          <span class="min-w-0 flex-1 truncate text-left">{{ t("albums.aiShowOriginalThumbs") }}</span>
        </label>
        <label v-if="person && hasMultipleCenters" class="mobile-sheet-action">
          <input v-model="showCenters" type="checkbox" class="h-4 w-4 accent-amber-500" />
          <span class="min-w-0 flex-1 truncate text-left">{{ t("people.showNaturalClusters") }}</span>
        </label>
        <button
          v-for="action in menuActions"
          :key="action.id"
          type="button"
          class="mobile-sheet-action"
          :class="action.danger ? 'is-danger' : ''"
          :disabled="action.disabled"
          @click="pick(action)"
        >
          <component :is="action.icon" v-if="action.icon" class="h-5 w-5 shrink-0 opacity-80" />
          <span class="min-w-0 flex-1 truncate text-left">{{ action.label }}</span>
        </button>
      </template>
    </ActionSheet>

    <ActionSheet
      :show="mergeOpen"
      stacked
      :title="t('people.merge')"
      :actions="mergeActions"
      @close="mergeOpen = false"
      @select="(action) => mergeCluster(action.id)"
    />

    <AppDialog :show="nameOpen" :z-index="130" @close="nameOpen = false">
      <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-md rounded-2xl border p-4" :class="ui.isDark ? 'text-stone-100' : 'text-stone-900'">
        <h3 class="mb-3 text-sm font-semibold">{{ person ? t("people.rename") : t("albums.aiNameFace") }}</h3>
        <input
          v-model="nameValue"
          type="text"
          class="mb-4 w-full rounded-lg border px-3 py-2 text-sm"
          :class="ui.isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
          :placeholder="t('people.namePlaceholder')"
          @keyup.enter="saveName"
        />
        <div class="flex justify-end gap-2">
          <button type="button" class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="ui.isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="nameOpen = false">{{ t("albums.cancel") }}</button>
          <button type="button" class="rounded-xl bg-amber-600 px-3 py-1.5 text-xs font-medium text-white" @click="saveName">{{ t("albums.save") }}</button>
        </div>
      </div>
    </AppDialog>

    <AppDialog :show="!!removeTarget" :z-index="130" @close="removeTarget = null">
      <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-md rounded-2xl border p-4" :class="ui.isDark ? 'text-stone-100' : 'text-stone-900'">
        <h3 class="mb-2 text-sm font-semibold">{{ t("people.removeCluster") }}</h3>
        <p class="mb-4 text-xs leading-relaxed" :class="ui.isDark ? 'text-stone-400' : 'text-stone-600'">
          {{ t("people.removeClusterConfirm") }}
        </p>
        <div class="flex justify-end gap-2">
          <button type="button" class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="ui.isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="removeTarget = null">{{ t("albums.cancel") }}</button>
          <button type="button" class="rounded-xl bg-red-600 px-3 py-1.5 text-xs font-medium text-white" @click="confirmRemoveCenter">{{ t("people.removeCluster") }}</button>
        </div>
      </div>
    </AppDialog>

    <AppDialog :show="deleteOpen" :z-index="130" @close="deleteOpen = false">
      <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-md rounded-2xl border p-4" :class="ui.isDark ? 'text-stone-100' : 'text-stone-900'">
        <h3 class="mb-2 text-sm font-semibold">{{ t("people.delete") }}</h3>
        <p class="mb-4 text-xs leading-relaxed" :class="ui.isDark ? 'text-stone-400' : 'text-stone-600'">
          {{ t("people.deleteConfirm", { name: personLabel(person) }) }}
        </p>
        <div class="flex justify-end gap-2">
          <button type="button" class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="ui.isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="deleteOpen = false">{{ t("albums.cancel") }}</button>
          <button type="button" class="rounded-xl bg-red-600 px-3 py-1.5 text-xs font-medium text-white" @click="confirmDelete">{{ t("people.delete") }}</button>
        </div>
      </div>
    </AppDialog>
  </Screen>
</template>
