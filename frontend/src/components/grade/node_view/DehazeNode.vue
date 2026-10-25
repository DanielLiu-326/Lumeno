<script setup>
import NodeField from "./NodeField.vue";

defineProps({
  node: { type: Object, required: true },
  isDark: { type: Boolean, default: false },
});

const emit = defineEmits(["slider", "choice", "toggle"]);
</script>

<template>
  <!-- dehaze -->
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
