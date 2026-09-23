<script setup>
import { computed, inject, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import AppDialog from "../../components/AppDialog.vue";
import { t } from "../../i18n/index.js";
import { IconArrowLeft, IconDots, IconTrash } from "../../utils/icons";
import { ALBUM_APP_KEY, MOBILE_UI_KEY } from "../context.js";
import FacePhotoGrid from "../faces/FacePhotoGrid.vue";
import { centerLabel, detailItemBox, faceItemToPhoto, personCenters } from "../faces/faceModel.js";
import { albumFaceLocation, albumLocation } from "../router.js";
import Screen from "../layouts/Screen.vue";
import ActionSheet from "../sheets/ActionSheet.vue";

defineOptions({ name: "FaceCenterScreen" });

const app = inject(ALBUM_APP_KEY);
const ui = inject(MOBILE_UI_KEY);
const route = useRoute();
const router = useRouter();

const showOriginal = ref(false);
const menuOpen = ref(false);
const removeOpen = ref(false);

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
const centerId = computed(() => queryValue("center"));
const open = computed(() => route.name === "album-face-center");

const person = computed(() =>
  (app.albumPeople || []).find((item) => item.id === personId.value) || null,
);
const center = computed(() => personCenters(person.value).find((item) => item.center_id === centerId.value) || null);
const albumName = computed(() => app.currentAlbum || dirName.value);
const albumOwner = computed(() => app.albumFacesOwner || app.currentUser || "");
const editable = computed(() => !!app.albumFacesEditable);
const title = computed(() => (center.value ? centerLabel(center.value) : t("people.clusterFaceCount", { count: 0 })));

const menuActions = computed(() => [
  {
    id: "remove",
    label: t("people.removeCluster"),
    icon: IconTrash,
    danger: true,
    disabled: !editable.value,
  },
]);

watch(
  () => [route.name, dirName.value, personId.value, centerId.value],
  async ([name, dir, personKey, centerKey]) => {
    if (name !== "album-face-center") return;
    if (!dir || !personKey || !centerKey) {
      router.replace(dir ? albumFaceLocation(dir, { person: personKey }) : { name: "albums" });
      return;
    }
    if (app.currentAlbum !== dir) await app.selectAlbum(dir);
  },
  { immediate: true },
);

function goBack() {
  ui.markPushClose?.();
  if (window.history.length > 1) {
    router.back();
    return;
  }
  router.replace(albumFaceLocation(dirName.value, { person: personId.value }));
}

function openPhoto(item) {
  const items = center.value?.appearances || [];
  const navPhotos = items.map((entry) => faceItemToPhoto(entry, albumName.value, albumOwner.value)).filter(Boolean);
  const photo = faceItemToPhoto(item, albumName.value, albumOwner.value);
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

function onMenu(action) {
  menuOpen.value = false;
  if (action?.id === "remove" && !action.disabled) removeOpen.value = true;
}

function confirmRemove() {
  if (!person.value?.id || !centerId.value) return;
  removeOpen.value = false;
  app.removeAlbumPersonCenter?.({ person_id: person.value.id, center_id: centerId.value });
  goBack();
}
</script>

<template>
  <Screen v-if="open" class="h-full">
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

    <div class="flex min-h-0 flex-1 flex-col">
      <p class="shrink-0 px-3 pt-3 text-[11px]" :class="ui.isDark ? 'text-stone-500' : 'text-stone-400'">
        {{ t("people.photoCount", { count: center?.appearance_count || 0 }) }}
      </p>
      <FacePhotoGrid
        :items="center?.appearances || []"
        :is-dark="ui.isDark"
        :album-name="albumName"
        :album-owner="albumOwner"
        :person="person"
        :editable="editable"
        :show-original="showOriginal"
        @open="openPhoto"
        @set-cover="setCover"
        @clear-cover="clearCover"
      />
    </div>

    <ActionSheet :show="menuOpen" :title="title" @close="menuOpen = false" @select="onMenu">
      <template #default="{ pick }">
        <label class="mobile-sheet-action">
          <input v-model="showOriginal" type="checkbox" class="h-4 w-4 accent-amber-500" />
          <span class="min-w-0 flex-1 truncate text-left">{{ t("albums.aiShowOriginalThumbs") }}</span>
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

    <AppDialog :show="removeOpen" :z-index="130" @close="removeOpen = false">
      <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-md rounded-2xl border p-4" :class="ui.isDark ? 'text-stone-100' : 'text-stone-900'">
        <h3 class="mb-2 text-sm font-semibold">{{ t("people.removeCluster") }}</h3>
        <p class="mb-4 text-xs leading-relaxed" :class="ui.isDark ? 'text-stone-400' : 'text-stone-600'">
          {{ t("people.removeClusterConfirm") }}
        </p>
        <div class="flex justify-end gap-2">
          <button type="button" class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="ui.isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="removeOpen = false">{{ t("albums.cancel") }}</button>
          <button type="button" class="rounded-xl bg-red-600 px-3 py-1.5 text-xs font-medium text-white" @click="confirmRemove">{{ t("people.removeCluster") }}</button>
        </div>
      </div>
    </AppDialog>
  </Screen>
</template>
