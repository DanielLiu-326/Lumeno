<script setup>
import { ref } from "vue";
import {
  IconCamera, IconChevronRight, IconFile, IconFilter, IconImage, IconStar,
} from "../utils/icons";
import { t } from "../i18n/index.js";

const props = defineProps({
  collections: { type: Array, default: () => [] },
  selectedId: { type: String, default: "all" },
  isDark: { type: Boolean, default: false },
});

const emit = defineEmits(["select"]);

// Groups that are expanded (open). Date is expanded by default.
const expanded = ref(new Set(["date"]));

function toggleGroup(id) {
  const next = new Set(expanded.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  expanded.value = next;
}

function selectNode(node) {
  if (node.filter != null) {
    emit("select", node);
  } else {
    toggleGroup(node.id);
  }
}

function isExpanded(id) {
  return expanded.value.has(id);
}

function nodeLabel(node) {
  const keys = {
    all: "collections.all",
    date: "collections.byDate",
    camera: "collections.byCamera",
    lens: "collections.byLens",
    tag: "collections.byTag",
    rating: "collections.byRating",
    color: "collections.byColor",
    format: "collections.byFormat",
  };
  return keys[node.id] ? t(keys[node.id]) : node.label;
}

const COLOR_SWATCHES = {
  red: "bg-red-500",
  yellow: "bg-yellow-400",
  green: "bg-emerald-500",
  blue: "bg-blue-500",
  purple: "bg-purple-500",
};
</script>

<template>
  <aside
    class="app-scrollbar flex w-48 shrink-0 flex-col overflow-y-auto border-r py-2"
    :class="isDark ? 'border-stone-700/60 bg-stone-900/60' : 'border-stone-200/60 bg-stone-50/80'"
  >
    <p class="mb-1 px-3 text-[10px] font-bold uppercase tracking-wider"
      :class="isDark ? 'text-stone-500' : 'text-stone-400'">
      {{ t('collections.title') }}
    </p>

    <template v-for="node in collections" :key="node.id">
      <!-- ── Top-level selectable leaf (All Photos) ── -->
      <template v-if="!node.children || node.children.length === 0">
        <button
          class="flex w-full items-center gap-2 rounded-lg px-3 py-1.5 text-left text-xs font-medium transition"
          :class="selectedId === node.id
            ? (isDark ? 'bg-amber-500/25 text-amber-200' : 'bg-amber-100 text-amber-900')
            : (isDark ? 'text-stone-300 hover:bg-stone-800 hover:text-stone-100' : 'text-stone-600 hover:bg-stone-200 hover:text-stone-900')"
          @click="selectNode(node)"
        >
          <IconImage class="h-3.5 w-3.5 shrink-0" />
          <span class="min-w-0 flex-1 truncate">{{ nodeLabel(node) }}</span>
          <span class="shrink-0 tabular-nums text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
            {{ node.count }}
          </span>
        </button>
      </template>

      <!-- ── Group with children ── -->
      <template v-else>
        <!-- Group header (expand/collapse) -->
        <button
          class="flex w-full items-center gap-1.5 px-3 py-1.5 text-left text-[10px] font-semibold uppercase tracking-wide transition"
          :class="isDark ? 'text-stone-400 hover:text-stone-200' : 'text-stone-500 hover:text-stone-700'"
          @click="toggleGroup(node.id)"
        >
          <!-- Chevron -->
          <svg class="h-3 w-3 shrink-0 transition-transform" :class="isExpanded(node.id) ? 'rotate-90' : ''"
            viewBox="0 0 20 20" fill="currentColor">
            <path fill-rule="evenodd" clip-rule="evenodd" d="M7.293 14.707a1 1 0 010-1.414L10.586 10 7.293 6.707a1 1 0 011.414-1.414l4 4a1 1 0 010 1.414l-4 4a1 1 0 01-1.414 0z"/>
          </svg>
          <!-- Group icon -->
          <IconCamera v-if="node.id === 'camera'" class="h-3 w-3 shrink-0" />
          <IconStar v-else-if="node.id === 'rating'" class="h-3 w-3 shrink-0" />
          <IconFile v-else-if="node.id === 'format'" class="h-3 w-3 shrink-0" />
          <IconFilter v-else-if="node.id === 'tag'" class="h-3 w-3 shrink-0" />
          <!-- Date icon -->
          <svg v-else-if="node.id === 'date'" class="h-3 w-3 shrink-0" viewBox="0 0 20 20" fill="currentColor">
            <path fill-rule="evenodd" clip-rule="evenodd" d="M6 2a1 1 0 00-1 1v1H4a2 2 0 00-2 2v10a2 2 0 002 2h12a2 2 0 002-2V6a2 2 0 00-2-2h-1V3a1 1 0 10-2 0v1H7V3a1 1 0 00-1-1zm0 5a1 1 0 000 2h8a1 1 0 100-2H6z"/>
          </svg>
          <!-- Lens icon -->
          <svg v-else-if="node.id === 'lens'" class="h-3 w-3 shrink-0" viewBox="0 0 20 20" fill="currentColor">
            <path fill-rule="evenodd" clip-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zM7 9H5v2h2V9zm8 0h-2v2h2V9zM9 9h2v2H9V9z"/>
          </svg>
          <!-- Color icon -->
          <svg v-else-if="node.id === 'color'" class="h-3 w-3 shrink-0" viewBox="0 0 20 20" fill="currentColor">
            <path d="M10 3.5a1.5 1.5 0 013 0V4a1 1 0 001 1h3a1 1 0 011 1v3a1 1 0 01-1 1h-.5a1.5 1.5 0 000 3h.5a1 1 0 011 1v3a1 1 0 01-1 1h-3a1 1 0 01-1-1v-.5a1.5 1.5 0 00-3 0v.5a1 1 0 01-1 1H6a1 1 0 01-1-1v-3a1 1 0 011-1h.5a1.5 1.5 0 000-3H6a1 1 0 01-1-1V6a1 1 0 011-1h3a1 1 0 001-1v-.5z"/>
          </svg>

          <span class="flex-1 truncate">{{ nodeLabel(node) }}</span>
          <span class="tabular-nums text-[10px]">{{ node.count }}</span>
        </button>

        <!-- Children (shown when expanded) -->
        <template v-if="isExpanded(node.id)">
          <template v-for="child in node.children" :key="child.id">
            <!-- Year node (may have month children) -->
            <template v-if="child.children && child.children.length > 0">
              <button
                class="flex w-full items-center gap-2 rounded-lg pl-6 pr-3 py-1 text-left text-xs transition"
                :class="selectedId === child.id
                  ? (isDark ? 'bg-amber-500/25 text-amber-200 font-medium' : 'bg-amber-100 text-amber-900 font-medium')
                  : (isDark ? 'text-stone-300 hover:bg-stone-800 hover:text-stone-100' : 'text-stone-600 hover:bg-stone-200 hover:text-stone-900')"
                @click="selectNode(child)"
              >
                <svg class="h-3 w-3 shrink-0 transition-transform" :class="isExpanded(child.id) ? 'rotate-90' : ''"
                  viewBox="0 0 20 20" fill="currentColor">
                  <path fill-rule="evenodd" clip-rule="evenodd" d="M7.293 14.707a1 1 0 010-1.414L10.586 10 7.293 6.707a1 1 0 011.414-1.414l4 4a1 1 0 010 1.414l-4 4a1 1 0 01-1.414 0z"/>
                </svg>
                <span class="min-w-0 flex-1 truncate">{{ child.label }}</span>
                <span class="shrink-0 tabular-nums text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                  {{ child.count }}
                </span>
              </button>
              <!-- Month sub-children -->
              <template v-if="isExpanded(child.id)">
                <button
                  v-for="grandchild in child.children"
                  :key="grandchild.id"
                  class="flex w-full items-center gap-2 rounded-lg pl-10 pr-3 py-0.5 text-left text-xs transition"
                  :class="selectedId === grandchild.id
                    ? (isDark ? 'bg-amber-500/25 text-amber-200 font-medium' : 'bg-amber-100 text-amber-900 font-medium')
                    : (isDark ? 'text-stone-400 hover:bg-stone-800 hover:text-stone-200' : 'text-stone-500 hover:bg-stone-200 hover:text-stone-800')"
                  @click="selectNode(grandchild)"
                >
                  <span class="min-w-0 flex-1 truncate">{{ grandchild.label }}</span>
                  <span class="shrink-0 tabular-nums text-[10px]" :class="isDark ? 'text-stone-600' : 'text-stone-400'">
                    {{ grandchild.count }}
                  </span>
                </button>
              </template>
            </template>

            <!-- Leaf child -->
            <button
              v-else
              class="flex w-full items-center gap-2 rounded-lg pl-7 pr-3 py-1 text-left text-xs transition"
              :class="selectedId === child.id
                ? (isDark ? 'bg-amber-500/25 text-amber-200 font-medium' : 'bg-amber-100 text-amber-900 font-medium')
                : (isDark ? 'text-stone-300 hover:bg-stone-800 hover:text-stone-100' : 'text-stone-600 hover:bg-stone-200 hover:text-stone-900')"
              @click="selectNode(child)"
            >
              <!-- Color dot for color collection items -->
              <span v-if="node.id === 'color' && COLOR_SWATCHES[child.id.split(':')[1]]"
                class="h-2.5 w-2.5 shrink-0 rounded-full"
                :class="COLOR_SWATCHES[child.id.split(':')[1]]">
              </span>
              <span class="min-w-0 flex-1 truncate">{{ child.label }}</span>
              <span class="shrink-0 tabular-nums text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                {{ child.count }}
              </span>
            </button>
          </template>
        </template>
      </template>
    </template>
  </aside>
</template>