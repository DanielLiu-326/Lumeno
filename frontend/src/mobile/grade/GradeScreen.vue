<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { t } from "../../i18n/index.js";
import { useGradeSession } from "../../composables/useGradeSession";
import { useOverlayHistory } from "../useOverlayHistory.js";
import { IconArrowLeft, IconCheck } from "../../utils/icons";
import GradeSlider from "./GradeSlider.vue";

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

const GROUPS = [
  { key: "tone", label: "grade.group.tone", ids: ["exposure", "local-contrast", "sigmoid"] },
  { key: "color", label: "grade.group.color", ids: ["velvia"] },
  { key: "correct", label: "grade.group.correct", ids: ["dehaze", "denoise", "sharpen"] },
];
const group = ref("tone");

const whiteBalance = computed(() => nodes.value.find((entry) => entry.id === "white-balance") || null);
const visibleNodes = computed(() => {
  const entries = activeNodes.value.slice();
  if (whiteBalance.value) entries.unshift(whiteBalance.value);
  return entries;
});

const activeNodes = computed(() => {
  const ids = GROUPS.find((entry) => entry.key === group.value)?.ids || [];
  return ids.map((id) => node(id)).filter(Boolean);
});

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

function onInput(node, field, value) {
  setParam(node.id, field.key, value);
}

function onChoice(node, field, value) {
  setChoice(node.id, field.key, value);
}

function modeOptions(node) {
  return node.fields.find((field) => field.key === "mode")?.options || [];
}

function resetField(node, field) {
  const fallback = node.defaults?.[field.key];
  if (fallback !== undefined) setParam(node.id, field.key, fallback);
}

function resetNode(node) {
  const defaults = node.defaults || {};
  beginEdit(node.id);
  for (const [key, value] of Object.entries(defaults)) {
    setParam(node.id, key, value);
  }
  endEdit();
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
      x: (a.x + b.x) / 2,
      y: (a.y + b.y) / 2,
      tx: view.x,
      ty: view.y,
    };
    clearTimeout(longTimer);
  }
}

function onPointerMove(event) {
  if (!pointers.has(event.pointerId)) return;
  pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
  const start = panStart;
  if (start && Math.hypot(event.clientX - start.x, event.clientY - start.y) > 6) {
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
  if (pointers.size === 1 && start && moved && view.scale > 1) {
    view.x = start.tx + (event.clientX - start.x);
    view.y = start.ty + (event.clientY - start.y);
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

      <section class="rounded-t-2xl border-t border-white/10 bg-[#151515] pb-[env(safe-area-inset-bottom)]">
        <div class="flex gap-1 overflow-x-auto px-2 pt-1.5">
          <button
            v-for="entry in GROUPS"
            :key="entry.key"
            type="button"
            class="shrink-0 rounded-lg px-3 py-1.5 text-[0.75rem] font-medium transition"
            :class="group === entry.key ? 'bg-white/14 text-white' : 'text-stone-400'"
            @click="group = entry.key"
          >
            {{ t(entry.label) }}
          </button>
        </div>

        <div class="max-h-[42vh] space-y-4 overflow-y-auto px-4 pb-4 pt-3">
          <div v-if="!canWrite" class="rounded-lg bg-rose-500/15 px-3 py-2 text-[0.75rem] text-rose-200">
            {{ t("mobile.gradeReadOnly") }}
          </div>

          <!-- One keyed list for every node, so a slider can never be patched
               against the wrong node when the group or the mode changes. -->
          <div
            v-for="node in visibleNodes"
            :key="node.id"
            class="space-y-3 border-b border-white/8 pb-4 last:border-b-0 last:pb-0"
          >
            <div class="flex items-center justify-between">
              <span class="text-[0.8125rem] font-medium text-stone-200">{{ node.title }}</span>
              <button
                type="button"
                class="rounded-md px-2 py-1 text-[0.6875rem] text-stone-400"
                @click="resetNode(node)"
              >
                {{ t("mobile.gradeReset") }}
              </button>
            </div>

            <div v-if="node.id === 'white-balance'" class="flex gap-1.5">
              <button
                v-for="option in modeOptions(node)"
                :key="option.value"
                type="button"
                class="flex-1 rounded-lg px-2 py-1.5 text-[0.75rem]"
                :class="node.params.mode === option.value ? 'bg-white/16 text-white' : 'bg-white/6 text-stone-400'"
                @click="onChoice(node, { key: 'mode' }, option.value)"
              >
                {{ option.label }}
              </button>
            </div>

            <template v-for="field in node.fields" :key="node.id + ':' + field.key">
              <GradeSlider
                v-if="field.kind === 'slider'"
                :label="field.label"
                :model-value="Number(node.params[field.key])"
                :min="field.min"
                :max="field.max"
                :step="(field.max - field.min) / 400"
                :display="formatValue(field, node.params[field.key])"
                :gradient="gradientFor(field)"
                :disabled="!canWrite"
                @start="beginEdit(node.id)"
                @input="(value) => onInput(node, field, value)"
                @reset="resetField(node, field)"
                @end="endEdit()"
              />
              <div v-else-if="field.kind === 'choice' && node.id !== 'white-balance'" class="flex gap-1.5">
                <button
                  v-for="option in field.options"
                  :key="option.value"
                  type="button"
                  class="flex-1 rounded-lg px-2 py-1.5 text-[0.75rem]"
                  :class="String(node.params[field.key]) === String(option.value) ? 'bg-white/16 text-white' : 'bg-white/6 text-stone-400'"
                  @click="onChoice(node, field, option.value)"
                >
                  {{ option.label }}
                </button>
              </div>
            </template>
          </div>
        </div>
      </section>
    </div>
  </Teleport>
</template>
