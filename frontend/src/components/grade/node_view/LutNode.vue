<script setup>
import { computed, onMounted, ref } from "vue";
import { t } from "../../../i18n/index.js";
import { httpJson } from "../../../utils/http";
import NodeField from "./NodeField.vue";

const props = defineProps({
  node: { type: Object, required: true },
  isDark: { type: Boolean, default: false },
});

const emit = defineEmits(["slider", "choice", "toggle", "pick"]);

const open = ref(false);
const library = ref([]);
const failed = ref("");

const ready = computed(() => library.value.filter((item) => item.status === "ready"));

const pickedName = computed(() => {
  const id = props.node.params?.lut || "";
  if (!id) return t("grade.lutNone");
  const found = library.value.find((item) => item.id === id);
  return found?.display_name || t("grade.lutNone");
});

async function loadLibrary() {
  failed.value = "";
  try {
    const payload = await httpJson("/api/grade/resources?kind=lut");
    library.value = payload.data?.items || [];
  } catch (err) {
    failed.value = err.message || t("grade.resourcesFailed");
  }
}

function toggleList() {
  open.value = !open.value;
  if (open.value) void loadLibrary();
}

function choose(item) {
  if (!item || item.id === props.node.params.lut) return;
  props.node.params.lut = item.id;
  const field = (props.node.fields || []).find((entry) => entry.key === "lut");
  if (field) field.display = item.display_name;
  open.value = false;
  emit("pick");
}

let wheelAcc = 0;

function onWheel(event) {
  event.preventDefault();
  event.stopPropagation();
  const list = ready.value;
  if (!list.length) {
    void loadLibrary();
    return;
  }
  const raw = event.deltaY || event.deltaX;
  const dy = event.deltaMode === 1 ? raw * 16 : event.deltaMode === 2 ? raw * 120 : raw;
  if (!dy) return;
  if (wheelAcc && Math.sign(dy) !== Math.sign(wheelAcc)) wheelAcc = 0;
  wheelAcc += dy;
  if (Math.abs(wheelAcc) < 40) return;
  const dir = wheelAcc > 0 ? 1 : -1;
  wheelAcc = 0;
  let index = list.findIndex((item) => item.id === props.node.params.lut);
  if (index < 0) index = dir > 0 ? -1 : list.length;
  choose(list[index + dir]);
}

onMounted(() => {
  void loadLibrary();
});
</script>

<template>
  <div class="flex items-center gap-2 py-1" @wheel="onWheel">
    <span class="min-w-16 shrink-0 text-[11px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">LUT</span>
    <span class="min-w-0 flex-1 truncate text-[11px]" :class="isDark ? 'text-stone-200' : 'text-stone-800'">{{ pickedName }}</span>
    <button type="button" class="shrink-0 rounded px-1.5 py-0.5 text-[11px]"
      :class="isDark ? 'bg-white/8 text-stone-300 hover:bg-white/12' : 'bg-white/80 text-stone-600 hover:bg-white'"
      @click.stop="toggleList">
      {{ t("grade.lutChoose") }}
    </button>
  </div>
  <div v-if="open" class="mb-1 max-h-40 overflow-y-auto rounded px-1 py-1"
    :class="isDark ? 'bg-black/20' : 'bg-white/70'">
    <p v-if="failed" class="px-1.5 py-1 text-[11px] text-red-500">{{ failed }}</p>
    <p v-else-if="ready.length === 0" class="px-1.5 py-1 text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-500'">{{ t("grade.lutEmpty") }}</p>
    <button v-for="item in ready" :key="item.id" type="button"
      class="flex w-full items-center rounded px-1.5 py-1 text-left text-[11px]"
      :class="item.id === node.params.lut
        ? (isDark ? 'bg-white/15 text-stone-50' : 'bg-stone-900 text-white')
        : (isDark ? 'text-stone-200 hover:bg-white/10' : 'text-stone-700 hover:bg-white')"
      @click.stop="choose(item)">
      <span class="min-w-0 flex-1 truncate">{{ item.display_name }}</span>
    </button>
  </div>
  <NodeField
    v-for="field in node.fields.filter((entry) => entry.key !== 'lut')"
    :key="field.key"
    :node="node"
    :field="field"
    :is-dark="isDark"
    @slider="emit('slider', $event)"
    @choice="emit('choice', $event)"
    @toggle="emit('toggle', $event)"
  />
</template>
