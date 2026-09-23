<script setup>
import { computed, inject, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import ViewSettingsDialog from "../../components/ViewSettingsDialog.vue";
import { t } from "../../i18n/index.js";
import { IconArrowLeft } from "../../utils/icons";
import { ALBUM_APP_KEY, MOBILE_UI_KEY } from "../context.js";
import { albumLocation } from "../router.js";
import Screen from "../layouts/Screen.vue";

defineOptions({ name: "ViewSettingsScreen" });

const app = inject(ALBUM_APP_KEY);
const ui = inject(MOBILE_UI_KEY);
const route = useRoute();
const router = useRouter();

const dirName = computed(() => {
  const raw = route.params.dirName;
  if (Array.isArray(raw)) return raw.filter(Boolean).join("/");
  return String(raw || "");
});

const open = computed(() => route.name === "album-view");
const layout = computed(() => ui.albumPanelRef?.viewLayout || null);

watch(
  () => [route.name, dirName.value],
  async ([name, dir]) => {
    if (name !== "album-view") return;
    if (!dir) {
      router.replace({ name: "albums" });
      return;
    }
    if (app.currentAlbum !== dir) await app.selectAlbum(dir);
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
  if (window.history.length > 1) {
    ui.markPushClose?.();
    router.back();
    return;
  }
  ui.markPushClose?.();
  goAlbum();
}

function onUpdate(next) {
  ui.albumPanelRef?.setViewLayout?.(next);
}
</script>

<template>
  <Screen class="h-full">
    <template #bar>
      <header class="mobile-topbar">
        <button type="button" class="mobile-icon-btn" @click="goBack">
          <IconArrowLeft class="h-5 w-5" />
        </button>
        <h1 class="min-w-0 flex-1 truncate text-base font-semibold">{{ t("view.settings") }}</h1>
      </header>
    </template>
    <ViewSettingsDialog
      v-if="layout"
      page
      :show="open"
      :model-value="layout"
      :is-dark="ui.isDark"
      :format-size="app.formatSize"
      @update:model-value="onUpdate"
      @close="goBack"
    />
  </Screen>
</template>
