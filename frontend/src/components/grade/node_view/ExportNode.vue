<script setup>
import { t } from "../../../i18n/index.js";
import NodeField from "./NodeField.vue";

defineProps({
  node: { type: Object, required: true },
  isDark: { type: Boolean, default: false },
});

const emit = defineEmits(["slider", "choice", "toggle"]);
</script>

<template>
  <!-- export -->
  <p
    class="px-2 py-2 text-xs leading-relaxed"
    :class="isDark ? 'text-stone-400' : 'text-stone-500'"
  >{{ t('grade.exportPlaceholder') }}</p>
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
</template>
