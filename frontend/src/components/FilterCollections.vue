<script setup>
import { IconImage } from "../utils/icons";
import { t } from "../i18n/index.js";
import { COLOR_SWATCH, colorKey, groupLabel } from "../composables/useAlbumFilters.js";

const props = defineProps({
  isDark: { type: Boolean, default: false },
  collections: { type: Array, default: () => [] },
  selectedCollectionId: { type: String, default: "all" },
  selectedGroupId: { type: String, default: "all" },
  currentGroup: { type: Object, default: null },
  treeExpanded: { type: Array, default: () => [] },
  showTitle: { type: Boolean, default: true },
  listClass: { type: String, default: "max-h-56" },
});

const emit = defineEmits(["update:selectedGroupId", "select-node"]);

function isExpanded(id) {
  return props.treeExpanded.includes(id);
}
</script>

<template>
  <div v-if="collections.length > 0">
    <p
      v-if="showTitle"
      class="mb-2 text-[11px] font-bold uppercase tracking-wider"
      :class="isDark ? 'text-stone-500' : 'text-stone-400'"
    >
      {{ t("collections.title") }}
    </p>

    <select
      :value="selectedGroupId"
      class="mb-2 w-full rounded-lg border px-2 py-1.5 text-xs app-control"
      :class="isDark ? 'text-stone-100' : 'text-stone-900'"
      @change="emit('update:selectedGroupId', $event.target.value)"
    >
      <option v-for="node in collections" :key="node.id" :value="node.id">
        {{ groupLabel(node.id) }}
      </option>
    </select>

    <div v-if="currentGroup" class="app-scrollbar flex flex-col gap-0.5 overflow-y-auto" :class="listClass">
      <template v-if="currentGroup.id === 'all'">
        <button
          type="button"
          class="flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-left text-xs font-medium transition"
          :class="selectedCollectionId === 'all'
            ? (isDark ? 'bg-amber-500/25 text-amber-200' : 'bg-amber-100 text-amber-900')
            : (isDark ? 'text-stone-300 hover:bg-white/8' : 'text-stone-600 hover:bg-black/5')"
          @click="emit('select-node', currentGroup)"
        >
          <IconImage class="h-3.5 w-3.5 shrink-0" />
          <span class="min-w-0 flex-1 truncate">{{ t("collections.all") }}</span>
          <span class="shrink-0 tabular-nums text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
            {{ currentGroup.count }}
          </span>
        </button>
      </template>

      <template v-else>
        <template v-for="child in (currentGroup.children || [])" :key="child.id">
          <template v-if="child.children && child.children.length > 0">
            <button
              type="button"
              class="flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-left text-xs transition"
              :class="selectedCollectionId === child.id
                ? (isDark ? 'bg-amber-500/25 text-amber-200 font-medium' : 'bg-amber-100 text-amber-900 font-medium')
                : (isDark ? 'text-stone-300 hover:bg-white/8' : 'text-stone-600 hover:bg-black/5')"
              @click="emit('select-node', child)"
            >
              <svg
                class="h-3 w-3 shrink-0 transition-transform"
                :class="isExpanded(child.id) ? 'rotate-90' : ''"
                viewBox="0 0 20 20"
                fill="currentColor"
              >
                <path fill-rule="evenodd" clip-rule="evenodd" d="M7.293 14.707a1 1 0 010-1.414L10.586 10 7.293 6.707a1 1 0 011.414-1.414l4 4a1 1 0 010 1.414l-4 4a1 1 0 01-1.414 0z"/>
              </svg>
              <span class="min-w-0 flex-1 truncate">{{ child.label }}</span>
              <span class="shrink-0 tabular-nums text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                {{ child.count }}
              </span>
            </button>
            <template v-if="isExpanded(child.id)">
              <button
                v-for="gc in child.children"
                :key="gc.id"
                type="button"
                class="flex w-full items-center gap-1.5 rounded-lg py-1 pl-7 pr-2 text-left text-xs transition"
                :class="selectedCollectionId === gc.id
                  ? (isDark ? 'bg-amber-500/25 text-amber-200 font-medium' : 'bg-amber-100 text-amber-900 font-medium')
                  : (isDark ? 'text-stone-400 hover:bg-white/8' : 'text-stone-500 hover:bg-black/5')"
                @click="emit('select-node', gc)"
              >
                <span class="min-w-0 flex-1 truncate">{{ gc.label }}</span>
                <span class="shrink-0 tabular-nums text-[11px]" :class="isDark ? 'text-stone-600' : 'text-stone-400'">
                  {{ gc.count }}
                </span>
              </button>
            </template>
          </template>

          <button
            v-else
            type="button"
            class="flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-left text-xs transition"
            :class="selectedCollectionId === child.id
              ? (isDark ? 'bg-amber-500/25 text-amber-200 font-medium' : 'bg-amber-100 text-amber-900 font-medium')
              : (isDark ? 'text-stone-300 hover:bg-white/8' : 'text-stone-600 hover:bg-black/5')"
            @click="emit('select-node', child)"
          >
            <span
              v-if="currentGroup.id === 'color' && COLOR_SWATCH[colorKey(child)]"
              class="h-3 w-3 shrink-0 rounded-full"
              :class="COLOR_SWATCH[colorKey(child)]"
            ></span>
            <span class="min-w-0 flex-1 truncate">{{ child.label }}</span>
            <span class="shrink-0 tabular-nums text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
              {{ child.count }}
            </span>
          </button>
        </template>
      </template>
    </div>
  </div>
</template>
