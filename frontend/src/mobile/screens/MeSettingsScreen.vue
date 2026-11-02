<script setup>
import { computed, inject, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import SettingsPanel from "../../components/SettingsPanel.vue";
import StoragePanel from "../../components/StoragePanel.vue";
import { t } from "../../i18n/index.js";
import { IconArrowLeft } from "../../utils/icons";
import { ALBUM_APP_KEY, MOBILE_UI_KEY } from "../context.js";
import Screen from "../layouts/Screen.vue";

defineOptions({ name: "MeSettingsScreen" });

const VALID = new Set(["preferences", "account", "password", "members", "appSettings", "webdav"]);

const app = inject(ALBUM_APP_KEY);
const ui = inject(MOBILE_UI_KEY);
const route = useRoute();
const router = useRouter();

const section = ref("preferences");
watch(
  () => [route.name, route.params.section],
  ([name, raw]) => {
    if (name !== "me-settings") return;
    const next = String(raw || "preferences");
    section.value = VALID.has(next) ? next : "preferences";
  },
  { immediate: true },
);

const title = computed(() => {
  const map = {
    preferences: "settings.preferences",
    account: "settings.accountInfo",
    password: "settings.password",
    members: "settings.memberManagement",
    appSettings: "settings.appSettings",
    webdav: "settings.webdavShares",
  };
  return t(map[section.value] || "nav.settings");
});

onMounted(() => {
  app.refreshAiStatus();
});

function goBack() {
  if (window.history.length > 1) {
    ui.markPushClose?.();
    router.back();
    return;
  }
  router.replace({ name: "me" });
}
</script>

<template>
  <Screen class="h-full">
    <template #bar>
      <header class="mobile-topbar">
        <button type="button" class="mobile-icon-btn" @click="goBack">
          <IconArrowLeft class="h-5 w-5" />
        </button>
        <h1 class="min-w-0 flex-1 truncate text-base font-semibold">{{ title }}</h1>
      </header>
    </template>
    <SettingsPanel
      hide-frame
      class="min-h-0 flex-1"
      :is-dark="ui.isDark"
      :initial-section="section"
      :members="app.members"
      :settings="app.settings"
      :ai-status="app.aiStatus"
      :is-admin="app.isAdmin"
      :current-user="app.currentUser"
      :current-locale="app.userLocale"
      :user-avatar="app.userAvatar"
      :wallpaper="app.userWallpaper"
      :accent="app.userAccent"
      :video-encode-order="app.userVideoEncodeOrder"
      :on-update-password="app.updatePassword"
      :on-upload-avatar="app.uploadAvatar"
      :on-upload-wallpaper="app.uploadWallpaper"
      :on-ai-index="app.queueAiIndex"
      :user-role="app.userRole"
      :webdav-shares="app.webdavShares"
      :on-webdav-update="app.updateWebdavShare"
      @update-settings="app.updateSettings"
      @scan-thumbnails="app.scanThumbnailsNow"
      @update-member="app.updateMember"
      @update-password="app.updatePassword"
      @update-preferences="app.updatePreferences"
      @preview-wallpaper="app.previewWallpaper"
      @preview-accent="app.previewAccent"
      @delete-member="app.deleteMember"
      @reset-member-password="({ username, new_password }) => app.resetMemberPassword(username, new_password)"
    >
      <template #storage>
        <StoragePanel
          embedded
          :is-dark="ui.isDark"
          :report="app.storageReport"
          :loading="app.storageLoading"
          :error="app.storageError"
          :stale="app.storageStale"
          :scanning="app.storageScanning"
          :cleaning="app.storageCleaning"
          :is-admin="app.isAdmin"
          :members="app.storageMembers"
          :members-loading="app.storageMembersLoading"
          :shared-cache="app.storageSharedCache"
          :viewing-owner="app.storageOwner"
          :on-view-member="app.viewStorageOwner"
          :on-scan-member="(owner) => app.startStorageScanFor(owner)"
          :on-scan-all="app.scanAllStorageMembers"
          :on-load-members="app.loadStorageMembers"
          :on-refresh="app.refreshStorageReport"
          :on-scan="app.startStorageScan"
          :on-cleanup="app.startStorageCleanup"
          :on-rebuild-index="app.rebuildAlbumIndex"
        />
      </template>
    </SettingsPanel>
  </Screen>
</template>
