<script setup>
import { computed, inject, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import AlbumAiToolsModal from "../../components/AlbumAiToolsModal.vue";
import { t } from "../../i18n/index.js";
import FacesBrowse from "../faces/FacesBrowse.vue";
import { IconArrowLeft, IconCamera, IconRefresh, IconSearch, IconWand } from "../../utils/icons";
import { ALBUM_APP_KEY, MOBILE_UI_KEY } from "../context.js";
import { albumAiSection, albumFaceLocation, albumLocation } from "../router.js";
import Screen from "../layouts/Screen.vue";

defineOptions({ name: "AiToolsScreen" });

const app = inject(ALBUM_APP_KEY);
const ui = inject(MOBILE_UI_KEY);
const route = useRoute();
const router = useRouter();
const modalRef = ref(null);

const dirName = computed(() => {
  const raw = route.params.dirName;
  if (Array.isArray(raw)) return raw.filter(Boolean).join("/");
  return String(raw || "");
});

const tool = computed(() => {
  if (route.name === "album-face" || route.name === "album-face-center") return "faces";
  return route.name === "album-ai" ? albumAiSection(route.query.section) : "";
});

const toolMeta = {
  faces: { labelKey: "albums.aiFaces", icon: IconCamera },
  duplicates: { labelKey: "albums.aiDuplicates", icon: IconWand },
  search: { labelKey: "albums.aiTextSearch", icon: IconSearch },
};

const toolTitle = computed(() => {
  const meta = toolMeta[tool.value];
  return meta ? t(meta.labelKey) : t("albums.aiTools");
});

watch(
  () => [route.name, dirName.value, tool.value],
  async ([name, dir, section], prev) => {
    if (name !== "album-ai") return;
    if (!dir || !section) {
      goAlbum();
      return;
    }
    const prevName = prev?.[0];
    if (prevName === "album-face" || prevName === "album-face-center") return;
    if (app.currentAlbum !== dir) await app.selectAlbum(dir);
    app.refreshPeopleForAlbum?.();
  },
  { immediate: true },
);

function goAlbum() {
  const dir = dirName.value;
  if (!dir) {
    router.replace({ name: "albums" });
    return;
  }
  router.replace(albumLocation(dir));
}

function goBack() {
  if (tool.value !== "faces" && modalRef.value?.handleBack?.()) return;
  if (window.history.length > 1) {
    ui.markPushClose?.();
    router.back();
    return;
  }
  ui.markPushClose?.();
  goAlbum();
}

function onOpenPhoto(payload) {
  ui.albumPanelRef?.openAiToolsPhoto?.(payload);
}

function onOpenPerson(person) {
  if (!person?.id) return;
  router.push(albumFaceLocation(dirName.value, { person: person.id }));
}

function onOpenCluster(cluster) {
  if (!cluster?.cluster_id) return;
  router.push(albumFaceLocation(dirName.value, { cluster: cluster.cluster_id }));
}
</script>

<template>
  <Screen class="h-full">
    <template #bar>
      <header class="mobile-topbar">
        <button type="button" class="mobile-icon-btn" @click="goBack">
          <IconArrowLeft class="h-5 w-5" />
        </button>
        <h1 class="min-w-0 flex-1 truncate text-base font-semibold">{{ toolTitle }}</h1>
        <button
          v-if="tool === 'faces'"
          type="button"
          class="mobile-icon-btn"
          :title="t('people.refresh')"
          @click="app.refreshPeopleForAlbum?.()"
        >
          <IconRefresh class="h-5 w-5" />
        </button>
      </header>
    </template>
    <FacesBrowse
      v-if="tool === 'faces'"
      :is-dark="ui.isDark"
      :album-name="app.currentAlbum"
      :album-owner="app.albumFacesOwner || app.currentUser"
      :busy="app.albumPeopleLoading"
      :hints-loading="app.albumFaceHintsLoading"
      :people="app.albumPeople"
      :suggestions="app.albumFaceSuggestionsDisplay"
      :editable="app.albumFacesEditable"
      @open-person="onOpenPerson"
      @open-cluster="onOpenCluster"
      @merge-cluster="app.mergeAlbumCluster"
      @patch-person="(id, updates) => app.patchAlbumPerson(id, updates)"
      @delete-person="(id) => app.deleteAlbumPerson(id)"
    />
    <AlbumAiToolsModal
      v-else-if="tool"
      ref="modalRef"
      page
      :show="true"
      :is-dark="ui.isDark"
      :is-mobile="true"
      :initial-section="tool"
      :album-name="app.currentAlbum"
      :album-owner="app.albumFacesOwner || app.currentUser"
      :busy="app.albumPeopleLoading"
      :hints-loading="app.albumFaceHintsLoading"
      :people="app.albumPeople"
      :suggestions="app.albumFaceSuggestionsDisplay"
      :editable="app.albumFacesEditable"
      :initial-query="ui.albumPanelRef?.textSearchInput || ''"
      :on-search-text="(opts) => app.searchAlbumByText({ ...opts, inModal: true })"
      :on-load-duplicates="app.loadAlbumDuplicatesForModal"
      @close="goAlbum"
      @back="goBack"
      @refresh="app.refreshPeopleForAlbum?.()"
      @adopt-cluster="app.adoptAlbumCluster"
      @merge-cluster="app.mergeAlbumCluster"
      @patch-person="(id, updates) => app.patchAlbumPerson(id, updates)"
      @remove-center="(body) => app.removeAlbumPersonCenter(body)"
      @delete-person="(id) => app.deleteAlbumPerson(id)"
      @open-photo="onOpenPhoto"
    />
  </Screen>
</template>
