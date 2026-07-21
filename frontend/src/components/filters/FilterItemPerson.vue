<script setup>
import { computed, ref } from "vue";
import { t } from "../../i18n/index.js";
import FaceAvatar from "../FaceAvatar.vue";

const props = defineProps({
  modelValue: { type: Object, required: true },
  isDark: { type: Boolean, default: false },
  avail: { type: Object, default: () => ({}) },
});
const emit = defineEmits(["update:modelValue", "apply"]);

const showUnnamed = ref(false);

const people = computed(() => {
  const list = props.avail.availablePeople || [];
  return list.filter((person) => {
    if (person?.hidden) return false;
    if (showUnnamed.value) return true;
    return String(person?.name || "").trim().length > 0;
  });
});

const selectedIds = computed(() =>
  Array.isArray(props.modelValue.personIds) ? props.modelValue.personIds : [],
);

function personLabel(person) {
  const name = String(person?.name || "").trim();
  return name || t("people.unnamed");
}

function isSelected(id) {
  return selectedIds.value.includes(id);
}

function togglePerson(person) {
  const id = person.id;
  const next = isSelected(id)
    ? selectedIds.value.filter((value) => value !== id)
    : [...selectedIds.value, id];
  emit("update:modelValue", { ...props.modelValue, personIds: next });
  emit("apply");
}
</script>

<template>
  <div class="flex max-h-48 flex-col gap-1 overflow-y-auto">
    <label class="mb-1 inline-flex items-center gap-2 text-[10px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
      <input v-model="showUnnamed" type="checkbox" class="rounded accent-amber-500" />
      {{ t("albums.aiShowUnnamedFaces") }}
    </label>
    <p v-if="people.length === 0" class="py-2 text-center text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
      {{ showUnnamed ? t("filter.personEmpty") : t("albums.aiNamedFacesEmpty") }}
    </p>
    <button
      v-for="person in people"
      :key="person.id"
      type="button"
      class="flex items-center gap-2 rounded-lg px-1.5 py-1 text-left transition"
      :class="isSelected(person.id)
        ? (isDark ? 'bg-amber-500/20 text-amber-100' : 'bg-amber-100 text-amber-900')
        : (isDark ? 'hover:bg-stone-800 text-stone-200' : 'hover:bg-stone-100 text-stone-700')"
      @click="togglePerson(person)"
    >
      <div class="h-8 w-8 shrink-0 overflow-hidden rounded-full" :class="isDark ? 'bg-stone-800' : 'bg-stone-200'">
        <FaceAvatar v-if="person.cover?.path" :cover="person.cover" :size="32" />
      </div>
      <div class="min-w-0 flex-1">
        <p class="truncate text-xs font-medium">{{ personLabel(person) }}</p>
        <p class="truncate text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
          {{ t("people.photoCount", { count: person.visible_photo_count ?? 0 }) }}
        </p>
      </div>
    </button>
  </div>
</template>
