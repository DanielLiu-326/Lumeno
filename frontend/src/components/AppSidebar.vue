<script setup>
import { computed } from "vue";
import { IconFolder, IconAlbum, IconGear, IconCalendarDay, IconHome, IconSparkles } from "../utils/icons";
import { t } from "../i18n/index.js";

const props = defineProps({
  isDark: {
    type: Boolean,
    default: false,
  },
  username: {
    type: String,
    required: true,
  },
  avatar: {
    type: String,
    default: null,
  },
  activeTab: {
    type: String,
    default: "dashboard",
  },
});

const emit = defineEmits(["switch-tab"]);

const avatarLetter = computed(() => (props.username ? props.username[0].toUpperCase() : "?"));

const tabs = computed(() => [
  { key: "dashboard", label: t("nav.dashboard"), icon: IconHome },
  { key: "albums", label: t("nav.albums"), icon: IconAlbum },
  { key: "files", label: t("nav.files"), icon: IconFolder },
  { key: "on_this_day", label: t("nav.on_this_day"), icon: IconCalendarDay },
  { key: "daily_picks", label: t("nav.daily_picks"), icon: IconSparkles },
]);
</script>

<template>
  <aside class="flex h-full w-56 flex-col border-r backdrop-blur" :class="props.isDark ? 'border-stone-800 bg-stone-950/70' : 'border-stone-200/80 bg-white/60'">
    <!-- Avatar / user section -->
    <div class="flex flex-col items-center gap-3 border-b px-4 py-6" :class="props.isDark ? 'border-stone-800' : 'border-stone-200/60'">
      <div class="h-14 w-14 overflow-hidden rounded-full shadow-md"
        :class="avatar ? '' : 'flex items-center justify-center bg-gradient-to-br from-amber-500 to-orange-600 text-xl font-bold text-white'">
        <img v-if="avatar" :src="`/api/avatar/${avatar}`" alt="avatar" class="h-full w-full object-cover" />
        <span v-else>{{ avatarLetter }}</span>
      </div>
      <span class="text-sm font-medium" :class="props.isDark ? 'text-stone-100' : 'text-stone-800'">{{ username }}</span>
    </div>

    <!-- Tabs -->
    <nav class="flex-1 px-3 py-4">
      <button
        v-for="tab in tabs"
        :key="tab.key"
        class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-sm font-medium transition"
        :class="activeTab === tab.key ? (props.isDark ? 'bg-amber-500/15 text-amber-200' : 'bg-amber-100/70 text-amber-900') : (props.isDark ? 'text-stone-300 hover:bg-stone-800 hover:text-stone-100' : 'text-stone-600 hover:bg-stone-100 hover:text-stone-800')"
        @click="emit('switch-tab', tab.key)"
      >
        <component :is="tab.icon" class="h-5 w-5 shrink-0" />
        {{ tab.label }}
      </button>
    </nav>

    <!-- Settings button -->
    <div class="border-t px-3 py-3" :class="props.isDark ? 'border-stone-800' : 'border-stone-200/60'">
      <button
        class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-sm font-medium transition"
        :class="activeTab === 'settings' ? (props.isDark ? 'bg-amber-500/15 text-amber-200' : 'bg-amber-100/70 text-amber-900') : (props.isDark ? 'text-stone-300 hover:bg-stone-800 hover:text-stone-100' : 'text-stone-600 hover:bg-stone-100 hover:text-stone-800')"
        @click="emit('switch-tab', 'settings')"
      >
        <IconGear class="h-5 w-5 shrink-0" />
        {{ t('nav.settings') }}
      </button>
    </div>
  </aside>
</template>
