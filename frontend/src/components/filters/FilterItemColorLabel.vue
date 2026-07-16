<script setup>
// Color swatch data lives here — the only place that knows about color labels
const COLOR_LABELS = [
  { key: "red",    color: "bg-red-500",     ring: "ring-red-400" },
  { key: "yellow", color: "bg-yellow-400",  ring: "ring-yellow-300" },
  { key: "green",  color: "bg-emerald-500", ring: "ring-emerald-400" },
  { key: "blue",   color: "bg-blue-500",    ring: "ring-blue-400" },
  { key: "purple", color: "bg-purple-500",  ring: "ring-purple-400" },
];

const props = defineProps({
  modelValue: { type: Object, required: true },
  isDark:     { type: Boolean, default: false },
  avail:      { type: Object, default: () => ({}) },
});
const emit = defineEmits(["update:modelValue", "apply"]);

function toggle(key) {
  const colorLabels = [...(props.modelValue.colorLabels || [])];
  const idx = colorLabels.indexOf(key);
  if (idx >= 0) colorLabels.splice(idx, 1);
  else colorLabels.push(key);
  emit("update:modelValue", { ...props.modelValue, colorLabels });
  emit("apply");
}
</script>

<template>
  <div class="flex gap-1.5">
    <button
      v-for="cl in COLOR_LABELS"
      :key="cl.key"
      class="h-5 w-5 rounded-full transition"
      :class="[
        cl.color,
        (modelValue.colorLabels || []).includes(cl.key)
          ? `ring-2 ${cl.ring} ring-offset-1`
          : 'opacity-60 hover:opacity-100',
      ]"
      @click="toggle(cl.key)"
    />
  </div>
</template>
