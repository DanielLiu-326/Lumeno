<script setup>
import { ref } from "vue";
import { t } from "../../../i18n/index.js";
import NodeField from "./NodeField.vue";

defineProps({
  node: { type: Object, required: true },
  isDark: { type: Boolean, default: false },
});

const emit = defineEmits(["slider", "choice", "toggle", "import-profile"]);
const fileRef = ref(null);

function onFile(event) {
  const file = event.target.files && event.target.files[0];
  event.target.value = "";
  if (file) emit("import-profile", file);
}
</script>

<template>
  <NodeField
    v-for="field in node.fields"
    :key="field.key"
    :node="node"
    :field="field"
    :is-dark="isDark"
    @slider="emit('slider', $event)"
    @choice="emit('choice', $event)"
    @toggle="emit('toggle', $event)"
  />
  <div v-if="node.params.camera" class="flex items-center gap-2 py-1">
    <span class="w-16 shrink-0"></span>
    <button type="button" class="rounded px-1.5 py-0.5 text-[11px]"
      :class="isDark ? 'bg-white/8 text-stone-300 hover:bg-white/12' : 'bg-white/80 text-stone-600 hover:bg-white'"
      @click="fileRef.click()">
      {{ t("grade.importProfile") }}
    </button>
    <input ref="fileRef" type="file" class="hidden" accept=".icc,.icm,.json,application/json" @change="onFile" />
  </div>
</template>
