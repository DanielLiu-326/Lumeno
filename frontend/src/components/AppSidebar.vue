<script setup>
import { computed } from "vue";
import { IconFolder } from "../utils/icons";

const props = defineProps({
  isDark: {
    type: Boolean,
    default: false,
  },
  username: {
    type: String,
    required: true,
  },
  activeTab: {
    type: String,
    default: "files",
  },
});

const emit = defineEmits(["switch-tab"]);

const avatarLetter = computed(() => (props.username ? props.username[0].toUpperCase() : "?"));

const tabs = [
  { key: "files", label: "文件管理", icon: IconFolder },
];
</script>

<template>
  <aside class="flex h-full w-56 flex-col border-r backdrop-blur" :class="props.isDark ? 'border-stone-800 bg-stone-950/70' : 'border-stone-200/80 bg-white/60'">
    <!-- Avatar / user section -->
    <div class="flex flex-col items-center gap-3 border-b px-4 py-6" :class="props.isDark ? 'border-stone-800' : 'border-stone-200/60'">
      <div
        class="flex h-14 w-14 items-center justify-center rounded-full bg-gradient-to-br from-amber-500 to-orange-600 text-xl font-bold text-white shadow-md"
      >
        {{ avatarLetter }}
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

    <!-- Footer -->
    <div class="border-t px-4 py-3" :class="props.isDark ? 'border-stone-800' : 'border-stone-200/60'">
      <p class="text-[10px]" :class="props.isDark ? 'text-stone-500' : 'text-stone-400'">Rust + Axum + Vue</p>
    </div>
  </aside>
</template>
