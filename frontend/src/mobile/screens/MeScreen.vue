<script setup>
import { computed, inject, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { t } from "../../i18n/index.js";
import { IconBell, IconChevronRight, IconGear, IconLogout, IconMoon, IconSun, IconTasks, IconUser } from "../../utils/icons";
import { ALBUM_APP_KEY, MOBILE_UI_KEY } from "../context.js";
import Screen from "../layouts/Screen.vue";

defineOptions({ name: "MeScreen" });

const app = inject(ALBUM_APP_KEY);
const ui = inject(MOBILE_UI_KEY);
const router = useRouter();

const avatarFailed = ref(false);
const avatarUrl = computed(() => {
  const id = app.userAvatar;
  return id ? `/api/avatar/${encodeURIComponent(id)}` : "";
});
const avatarLetter = computed(() => (app.currentUser || "?").charAt(0).toUpperCase());
const showAvatarImg = computed(() => Boolean(avatarUrl.value) && !avatarFailed.value);

watch(avatarUrl, () => { avatarFailed.value = false; });

const appearanceItems = computed(() => [
  { key: "preferences", label: t("settings.preferences") },
]);
const accountItems = computed(() => [
  { key: "account", label: t("settings.accountInfo") },
  { key: "password", label: t("settings.password") },
]);
const adminItems = computed(() => {
  const items = [];
  if (app.isAdmin) {
    items.push({ key: "members", label: t("settings.memberManagement") });
    items.push({ key: "appSettings", label: t("settings.appSettings") });
  }
  items.push({ key: "webdav", label: t("settings.webdavShares") });
  return items;
});

function openSection(key) {
  router.push({ name: "me-settings", params: { section: key } });
}
</script>

<template>
  <Screen>
    <template #bar>
      <header class="mobile-topbar">
        <h1 class="flex-1 font-display text-xl font-semibold">{{ t("nav.me") }}</h1>
      </header>
    </template>

    <div class="mobile-scroll px-4 pb-8">
      <button type="button" class="mobile-profile" @click="openSection('account')">
        <div class="mobile-avatar" :class="showAvatarImg ? '' : 'is-placeholder'">
          <img
            v-if="showAvatarImg"
            :src="avatarUrl"
            alt=""
            class="h-full w-full object-cover"
            @error="avatarFailed = true"
          />
          <span v-else class="text-xl font-bold">{{ avatarLetter }}</span>
        </div>
        <div class="min-w-0 flex-1 text-left">
          <p class="truncate text-lg font-semibold">{{ app.currentUser }}</p>
          <p class="mt-0.5 text-sm opacity-60">{{ app.userRole || t("settings.accountInfo") }}</p>
        </div>
        <IconChevronRight class="h-5 w-5 opacity-40" />
      </button>

      <section class="mobile-group">
        <p class="mobile-group-label">{{ t("mobile.appearance") }}</p>
        <button type="button" class="mobile-row" @click="ui.toggleTheme()">
          <component :is="ui.isDark ? IconSun : IconMoon" class="h-5 w-5 opacity-70" />
          <span class="flex-1 text-left">{{ ui.isDark ? t("topbar.switchToLight") : t("topbar.switchToDark") }}</span>
        </button>
        <button v-for="item in appearanceItems" :key="item.key" type="button" class="mobile-row" @click="openSection(item.key)">
          <IconGear class="h-5 w-5 opacity-70" />
          <span class="flex-1 text-left">{{ item.label }}</span>
          <IconChevronRight class="h-5 w-5 opacity-30" />
        </button>
      </section>

      <section class="mobile-group">
        <p class="mobile-group-label">{{ t("mobile.account") }}</p>
        <button v-for="item in accountItems" :key="item.key" type="button" class="mobile-row" @click="openSection(item.key)">
          <IconUser class="h-5 w-5 opacity-70" />
          <span class="flex-1 text-left">{{ item.label }}</span>
          <IconChevronRight class="h-5 w-5 opacity-30" />
        </button>
      </section>

      <section class="mobile-group">
        <p class="mobile-group-label">{{ t("mobile.tasksAndNotify") }}</p>
        <button type="button" class="mobile-row" @click="ui.openTasks()">
          <IconTasks class="h-5 w-5 opacity-70" />
          <span class="flex-1 text-left">{{ t("topbar.tasks") }}</span>
          <span v-if="ui.activeTaskCount" class="rounded-full bg-rose-600 px-2 py-0.5 text-[11px] text-white">{{ ui.activeTaskCount }}</span>
        </button>
        <button type="button" class="mobile-row" @click="ui.openNotify()">
          <IconBell class="h-5 w-5 opacity-70" />
          <span class="flex-1 text-left">{{ t("topbar.notifications") }}</span>
          <span v-if="ui.unreadNotificationCount" class="rounded-full bg-rose-600 px-2 py-0.5 text-[11px] text-white">{{ ui.unreadNotificationCount }}</span>
        </button>
      </section>

      <section class="mobile-group">
        <p class="mobile-group-label">{{ t("mobile.admin") }}</p>
        <button v-for="item in adminItems" :key="item.key" type="button" class="mobile-row" @click="openSection(item.key)">
          <IconGear class="h-5 w-5 opacity-70" />
          <span class="flex-1 text-left">{{ item.label }}</span>
          <IconChevronRight class="h-5 w-5 opacity-30" />
        </button>
      </section>

      <button type="button" class="mobile-logout" @click="app.logout()">
        <IconLogout class="h-5 w-5" />
        {{ t("topbar.logout") }}
      </button>
    </div>
  </Screen>
</template>
