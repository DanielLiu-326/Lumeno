<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { t } from "../../i18n/index.js";
import { useGradeSession } from "../../composables/useGradeSession";
import { useOverlayHistory } from "../useOverlayHistory.js";
import { IconArrowLeft, IconCheck } from "../../utils/icons";
import GradeSlider from "./GradeSlider.vue";
import GradeToolIcon from "./GradeToolIcon.vue";
import Collapse from "./Collapse.vue";

const props = defineProps({
  photo: { type: Object, required: true },
});

const emit = defineEmits(["close"]);

const {
  nodes, preview, before, loading, error, saved, canWrite, canUndo, canRedo,
  node, open: openSession, setParam, setChoice, beginEdit, endEdit, undo, redo,
} = useGradeSession();

const open = ref(false);
useOverlayHistory(open, () => emit("close"));

// Bottom bar: one icon per category. Tapping it reveals the tools of that
// category; tapping a tool reveals its parameters. Nothing else opens.
const CATEGORIES = [
  { key: "tone", label: "grade.group.tone", ids: ["exposure", "local-contrast", "sigmoid"] },
  { key: "color", label: "grade.group.color", ids: ["velvia"] },
  { key: "correct", label: "grade.group.correct", ids: ["dehaze", "denoise", "sharpen"] },
  { key: "base", label: "grade.group.base", ids: ["white-balance"] },
];

const categories = computed(() =>
  CATEGORIES
    .map((entry) => ({ ...entry, nodes: entry.ids.map((id) => node(id)).filter(Boolean) }))
    .filter((entry) => entry.nodes.length > 0),
);

const category = ref("");
const toolId = ref("");

const categoryEntry = computed(() => categories.value.find((entry) => entry.key === category.value) || null);
const selectedNode = computed(() => categoryEntry.value?.nodes.find((entry) => entry.id === toolId.value) || null);

// The collapsed panel keeps its last content mounted, otherwise the height
// animation would jump to zero the moment the selection clears.
const lastNode = ref(null);
const lastCategory = ref(null);
watch(selectedNode, (entry) => {
  if (entry) lastNode.value = entry;
});
watch(categoryEntry, (entry) => {
  if (entry) lastCategory.value = entry;
});
const panelNode = computed(() => selectedNode.value || lastNode.value);
const toolsCategory = computed(() => categoryEntry.value || lastCategory.value);

watch(
  categories,
  (list) => {
    if (category.value && !list.some((entry) => entry.key === category.value)) {
      category.value = "";
      toolId.value = "";
    }
  },
  { immediate: true },
);

watch(categoryEntry, (entry) => {
  if (!entry) return;
  if (!entry.nodes.some((node) => node.id === toolId.value)) toolId.value = "";
});

function toggleCategory(key) {
  category.value = category.value === key ? "" : key;
  toolId.value = "";
}

function toggleTool(id) {
  toolId.value = toolId.value === id ? "" : id;
}

function modified(nodeEntry) {
  const defaults = nodeEntry.defaults || {};
  return Object.entries(nodeEntry.params || {}).some(([key, value]) => {
    const fallback = defaults[key];
    if (fallback === undefined) return false;
    return JSON.stringify(value) !== JSON.stringify(fallback);
  });
}

const TEMP_GRADIENT = "linear-gradient(90deg,#2f5fe0,#8fb3ff,#f2f2f2,#ffd479,#ffab2e)";
const TINT_GRADIENT = "linear-gradient(90deg,#3fbf7f,#d0d0d0,#b56ad6)";

function formatValue(field, value) {
  const number = Number(value);
  if (!Number.isFinite(number)) return "";
  if (field.key === "temperature") return `${Math.round(number)} K`;
  if (field.max - field.min <= 2) return number.toFixed(2);
  const rounded = Math.abs(number) >= 100 ? Math.round(number) : Number(number.toFixed(2));
  if (field.min < 0 && rounded > 0) return `+${rounded}`;
  return String(rounded);
}

function gradientFor(field) {
  if (field.key === "temperature") return TEMP_GRADIENT;
  if (field.key === "tint") return TINT_GRADIENT;
  return "";
}

function onInput(nodeEntry, field, value) {
  setParam(nodeEntry.id, field.key, value);
}

function onChoice(nodeEntry, field, value) {
  setChoice(nodeEntry.id, field.key, value);
}

function resetField(nodeEntry, field) {
  const fallback = nodeEntry.defaults?.[field.key];
  if (fallback !== undefined) setParam(nodeEntry.id, field.key, fallback);
}

function resetNode(nodeEntry) {
  const defaults = nodeEntry.defaults || {};
  beginEdit(nodeEntry.id);
  for (const [key, value] of Object.entries(defaults)) {
    setParam(nodeEntry.id, key, value);
  }
  endEdit();
}

function modeOptions(nodeEntry) {
  return nodeEntry.fields.find((field) => field.key === "mode")?.options || [];
}

// ── canvas ────────────────────────────────────────────
const stage = ref(null);
const view = reactive({ scale: 1, x: 0, y: 0 });
const comparing = ref(false);
const pointers = new Map();
let panStart = null;
let pinchStart = null;
let moved = false;
let longTimer = 0;
let lastTap = 0;
let downAt = 0;

const imageStyle = computed(() => ({
  transform: `translate3d(${view.x}px, ${view.y}px, 0) scale(${view.scale})`,
  transition: pointers.size ? "none" : "transform 160ms ease-out",
}));

function clampView() {
  const rect = stage.value?.getBoundingClientRect();
  if (!rect) return;
  const maxX = (rect.width * (view.scale - 1)) / 2;
  const maxY = (rect.height * (view.scale - 1)) / 2;
  view.x = Math.max(-maxX, Math.min(maxX, view.x));
  view.y = Math.max(-maxY, Math.min(maxY, view.y));
}

function onPointerDown(event) {
  event.preventDefault();
  stage.value?.setPointerCapture?.(event.pointerId);
  pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
  moved = false;
  downAt = Date.now();
  if (pointers.size === 1) {
    panStart = { x: event.clientX, y: event.clientY, tx: view.x, ty: view.y };
    longTimer = window.setTimeout(() => {
      if (!moved) comparing.value = true;
    }, 480);
  } else if (pointers.size === 2) {
    const [a, b] = [...pointers.values()];
    pinchStart = {
      distance: Math.hypot(a.x - b.x, a.y - b.y),
      scale: view.scale,
    };
    clearTimeout(longTimer);
  }
}

function onPointerMove(event) {
  if (!pointers.has(event.pointerId)) return;
  pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
  if (panStart && Math.hypot(event.clientX - panStart.x, event.clientY - panStart.y) > 6) {
    moved = true;
    clearTimeout(longTimer);
  }
  if (pointers.size === 2 && pinchStart) {
    const [a, b] = [...pointers.values()];
    const distance = Math.hypot(a.x - b.x, a.y - b.y);
    view.scale = Math.max(1, Math.min(4, (distance / pinchStart.distance) * pinchStart.scale));
    clampView();
    return;
  }
  if (pointers.size === 1 && panStart && moved && view.scale > 1) {
    view.x = panStart.tx + (event.clientX - panStart.x);
    view.y = panStart.ty + (event.clientY - panStart.y);
    clampView();
  }
}

function onPointerUp(event) {
  pointers.delete(event.pointerId);
  if (pointers.size === 1) {
    const [rest] = [...pointers.values()];
    panStart = { x: rest.x, y: rest.y, tx: view.x, ty: view.y };
    pinchStart = null;
    return;
  }
  if (pointers.size > 1) return;
  clearTimeout(longTimer);
  comparing.value = false;
  pinchStart = null;
  panStart = null;
  const wasTap = !moved && Date.now() - downAt < 260;
  if (wasTap) {
    const now = Date.now();
    if (now - lastTap < 300) {
      if (view.scale > 1.2) {
        view.scale = 1;
        view.x = 0;
        view.y = 0;
      } else {
        view.scale = 2.5;
      }
      lastTap = 0;
    } else {
      lastTap = now;
    }
  }
}

onBeforeUnmount(() => {
  clearTimeout(longTimer);
});

onMounted(() => {
  open.value = true;
  void openSession(props.photo.path);
});

watch(
  () => props.photo.path,
  (path) => {
    if (path) void openSession(path);
  },
);

function close() {
  emit("close");
}
</script>

<template>
  <Teleport to="body">
    <div class="mobile-grade fixed inset-0 z-[160] flex flex-col bg-[#0d0d0d] text-stone-100">
      <header class="flex items-center gap-1 px-2 pt-[env(safe-area-inset-top)]">
        <button type="button" class="mobile-icon-btn flex h-11 w-11 items-center justify-center" :title="t('mobile.gradeClose')" @click="close">
          <IconArrowLeft class="h-5 w-5" />
        </button>
        <div class="min-w-0 flex-1 text-center">
          <p class="truncate text-[0.8125rem] font-medium">{{ photo.name || photo.path }}</p>
          <p class="text-[0.625rem]" :class="error ? 'text-rose-400' : 'text-stone-500'">
            {{ error || (saved ? t("mobile.gradeSaved") : t("mobile.gradeSaving")) }}
          </p>
        </div>
        <button type="button" class="mobile-icon-btn flex h-11 w-11 items-center justify-center text-lg" :disabled="!canUndo" :class="!canUndo ? 'opacity-30' : ''" :title="t('mobile.gradeUndo')" @click="undo()">↶</button>
        <button type="button" class="mobile-icon-btn flex h-11 w-11 items-center justify-center text-lg" :disabled="!canRedo" :class="!canRedo ? 'opacity-30' : ''" :title="t('mobile.gradeRedo')" @click="redo()">↷</button>
        <button type="button" class="mobile-icon-btn flex h-11 w-11 items-center justify-center" :title="t('mobile.gradeDone')" @click="close">
          <IconCheck class="h-5 w-5" />
        </button>
      </header>

      <div
        ref="stage"
        class="relative min-h-0 flex-1 touch-none overflow-hidden bg-black"
        @pointerdown="onPointerDown"
        @pointermove="onPointerMove"
        @pointerup="onPointerUp"
        @pointercancel="onPointerUp"
      >
        <img
          v-if="preview"
          :src="comparing ? before : preview"
          class="absolute inset-0 h-full w-full object-contain"
          :style="imageStyle"
          draggable="false"
          alt=""
        />
        <div v-else class="flex h-full items-center justify-center text-sm text-stone-500">
          {{ loading ? t("mobile.gradeLoading") : t("mobile.gradeEmpty") }}
        </div>
        <div class="pointer-events-none absolute inset-x-0 top-0 flex justify-center pt-2">
          <span v-if="comparing" class="rounded-full bg-black/60 px-2.5 py-1 text-[0.6875rem] text-stone-200">{{ t("mobile.gradeCompare") }}</span>
        </div>
      </div>

      <section class="border-t border-white/10 bg-[#151515] pb-[env(safe-area-inset-bottom)]">
        <div v-if="!canWrite" class="mx-4 mt-3 rounded-lg bg-rose-500/15 px-3 py-2 text-[0.75rem] text-rose-200">
          {{ t("mobile.gradeReadOnly") }}
        </div>

        <!-- parameters of the selected tool -->
        <Collapse :open="!!selectedNode" :duration="300">
            <div v-if="panelNode" class="max-h-[36vh] overflow-y-auto px-4 pt-3">
              <div :key="panelNode.id">
                  <div class="mb-3 flex items-center justify-between">
                    <span class="text-[0.8125rem] font-medium text-stone-200">{{ panelNode.title }}</span>
                    <button type="button" class="rounded-md px-2 py-1 text-[0.6875rem] text-stone-400" @click="resetNode(panelNode)">
                      {{ t("mobile.gradeReset") }}
                    </button>
                  </div>

                  <div v-if="panelNode.id === 'white-balance'" class="mb-3 flex gap-1.5">
                    <button
                      v-for="option in modeOptions(panelNode)"
                      :key="option.value"
                      type="button"
                      class="flex-1 rounded-lg px-2 py-1.5 text-[0.75rem]"
                      :class="panelNode.params.mode === option.value ? 'bg-white/16 text-white' : 'bg-white/6 text-stone-400'"
                      @click="onChoice(panelNode, { key: 'mode' }, option.value)"
                    >
                      {{ option.label }}
                    </button>
                  </div>

                  <div class="space-y-4 pb-3">
                    <template v-for="field in panelNode.fields" :key="panelNode.id + ':' + field.key">
                      <GradeSlider
                        v-if="field.kind === 'slider'"
                        :label="field.label"
                        :model-value="Number(panelNode.params[field.key])"
                        :min="field.min"
                        :max="field.max"
                        :step="(field.max - field.min) / 400"
                        :display="formatValue(field, panelNode.params[field.key])"
                        :gradient="gradientFor(field)"
                        :disabled="!canWrite"
                        @start="beginEdit(panelNode.id)"
                        @input="(value) => onInput(panelNode, field, value)"
                        @reset="resetField(panelNode, field)"
                        @end="endEdit()"
                      />
                      <div v-else-if="field.kind === 'choice' && panelNode.id !== 'white-balance'" class="flex gap-1.5">
                        <button
                          v-for="option in field.options"
                          :key="option.value"
                          type="button"
                          class="flex-1 rounded-lg px-2 py-1.5 text-[0.75rem]"
                          :class="String(panelNode.params[field.key]) === String(option.value) ? 'bg-white/16 text-white' : 'bg-white/6 text-stone-400'"
                          @click="onChoice(panelNode, field, option.value)"
                        >
                          {{ option.label }}
                        </button>
                      </div>
                    </template>

                  </div>
                </div>
            </div>
        </Collapse>

        <!-- tools of the selected category -->
        <Collapse :open="!!categoryEntry" :duration="250">
            <div v-if="toolsCategory" class="flex gap-1 overflow-x-auto px-3 pt-2">
              <button
                v-for="entry in toolsCategory.nodes"
                :key="entry.id"
                type="button"
                class="flex shrink-0 items-center gap-1.5 rounded-lg px-3 py-1.5 text-[0.75rem]"
                :class="toolId === entry.id ? 'bg-white/14 text-white' : 'text-stone-400'"
                @click="toggleTool(entry.id)"
              >
                <GradeToolIcon :kind="entry.id" :size="22" />
                {{ entry.title }}
                <span v-if="modified(entry)" class="h-1 w-1 rounded-full bg-sky-400" />
              </button>
            </div>
        </Collapse>

        <!-- categories -->
        <div class="flex items-stretch justify-around px-2 pb-1 pt-2">
          <button
            v-for="entry in categories"
            :key="entry.key"
            type="button"
            class="flex flex-1 flex-col items-center gap-1 rounded-lg py-1.5 text-[0.625rem]"
            :class="category === entry.key ? 'text-white' : 'text-stone-500'"
            @click="toggleCategory(entry.key)"
          >
            <GradeToolIcon :kind="entry.key" :size="18" />
            {{ t(entry.label) }}
          </button>
        </div>
      </section>
    </div>
  </Teleport>
</template>

