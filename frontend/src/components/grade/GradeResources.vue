<script setup>
import { onMounted, ref, watch } from "vue";
import { t } from "../../i18n/index.js";
import { httpJson, postJson } from "../../utils/http";
import { IconCheck, IconClose, IconFolder, IconImage, IconPencil, IconTrash } from "../../utils/icons";
import AppDialog from "../AppDialog.vue";

defineProps({
  isDark: { type: Boolean, default: false },
});

const emit = defineEmits(["close"]);

const navItems = [
  { id: "lut", labelKey: "grade.resourcesLut", icon: IconImage },
  { id: "preset", labelKey: "grade.resourcesPreset", icon: IconFolder },
];

const kind = ref("lut");
const items = ref([]);
const loading = ref(false);
const busy = ref(false);
const error = ref("");
const notice = ref("");
const renaming = ref("");
const renameText = ref("");
const confirmHash = ref("");
const fileRef = ref(null);

async function load() {
  loading.value = true;
  error.value = "";
  confirmHash.value = "";
  try {
    const payload = await httpJson(`/api/grade/resources?kind=${kind.value}`);
    items.value = payload.data?.items || [];
  } catch (err) {
    error.value = err.message || t("grade.resourcesFailed");
  } finally {
    loading.value = false;
  }
}

watch(kind, () => {
  notice.value = "";
  renaming.value = "";
  void load();
});

onMounted(() => {
  void load();
});

function pickFile() {
  fileRef.value?.click();
}

async function onFile(event) {
  const files = [...(event.target.files || [])];
  event.target.value = "";
  if (!files.length) return;
  busy.value = true;
  error.value = "";
  notice.value = "";
  const url = kind.value === "lut" ? "/api/grade/resources/lut" : "/api/grade/resources/preset";
  let existed = 0;
  try {
    for (const file of files) {
      const body = new FormData();
      body.append("file", file, file.name);
      const payload = await httpJson(url, { method: "POST", body });
      if (payload.data && payload.data.created === false) existed += 1;
    }
    if (existed) notice.value = t("grade.resourcesExists");
    await load();
  } catch (err) {
    error.value = err.message || t("grade.resourcesFailed");
  } finally {
    busy.value = false;
  }
}

function startRename(item) {
  renaming.value = item.hash;
  renameText.value = item.display_name;
  confirmHash.value = "";
}

async function saveRename(item) {
  const displayName = renameText.value.trim();
  if (!displayName || displayName === item.display_name) {
    renaming.value = "";
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    await postJson("/api/grade/resources/rename", {
      kind: kind.value,
      hash: item.hash,
      display_name: displayName,
    });
    renaming.value = "";
    await load();
  } catch (err) {
    error.value = err.message || t("grade.resourcesFailed");
  } finally {
    busy.value = false;
  }
}

async function remove(item) {
  if (item.ref_count > 0) return;
  if (confirmHash.value !== item.hash) {
    confirmHash.value = item.hash;
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    await postJson("/api/grade/resources/delete", {
      kind: kind.value,
      hash: item.hash,
    });
    confirmHash.value = "";
    await load();
  } catch (err) {
    error.value = err.message || t("grade.resourcesFailed");
  } finally {
    busy.value = false;
  }
}

function shortHash(hash) {
  return String(hash || "").slice(0, 8);
}
</script>

<template>
  <AppDialog :show="true" :z-index="110" center-class="p-0 sm:p-4" @close="emit('close')">
    <div class="app-dialog-shell flex w-full overflow-hidden"
      :class="[
        isDark ? 'text-stone-100' : 'text-stone-900',
        'h-[min(92vh,56rem)] max-h-full max-w-6xl rounded-2xl',
      ]">
      <nav class="app-frost-sidebar app-split-r flex w-56 shrink-0 flex-col">
        <div class="flex-1 px-3 py-4">
          <button v-for="item in navItems" :key="item.id" type="button"
            class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left text-sm font-medium transition"
            :class="kind === item.id
              ? (isDark ? 'bg-amber-500/15 text-amber-200' : 'bg-amber-100/70 text-amber-900')
              : (isDark ? 'text-stone-300 hover:bg-stone-800 hover:text-stone-100' : 'text-stone-600 hover:bg-stone-200 hover:text-stone-800')"
            @click="kind = item.id">
            <component :is="item.icon" class="h-5 w-5 shrink-0" />
            <span class="truncate">{{ t(item.labelKey) }}</span>
          </button>
        </div>
      </nav>
    <div class="app-dialog-panel app-dialog-panel-soft flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <div class="app-split-b flex items-center gap-2 px-3 py-2.5">
        <p class="min-w-0 flex-1 truncate text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
          {{ t(kind === 'lut' ? 'grade.resourcesLut' : 'grade.resourcesPreset') }}
        </p>
        <button type="button" class="rounded-full p-1.5 transition"
          :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
          :title="t('grade.resourcesClose')"
          @click="emit('close')">
          <IconClose class="h-3.5 w-3.5" />
        </button>
        <input ref="fileRef" type="file" multiple class="hidden"
          accept=".cube,text/plain"
          @change="onFile" />
      </div>
      <div class="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden p-3 sm:p-4">
      <div v-if="kind === 'lut'" class="mb-3 shrink-0">
        <button type="button" class="rounded-full px-3 py-1.5 text-xs font-medium transition"
          :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
          :disabled="busy"
          @click="pickFile">{{ t('grade.resourcesImport') }}</button>
      </div>
      <p v-if="error" class="shrink-0 px-3 pt-2 text-[12px] text-red-500">{{ error }}</p>
      <p v-else-if="notice" class="shrink-0 px-3 pt-2 text-[12px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ notice }}</p>
      <div class="app-scrollbar min-h-0 flex-1 overflow-y-auto">
        <p v-if="loading" class="px-2 py-6 text-center text-[12px]" :class="isDark ? 'text-stone-500' : 'text-stone-500'">{{ t('grade.loading') }}</p>
        <p v-else-if="items.length === 0" class="px-2 py-6 text-center text-[12px]" :class="isDark ? 'text-stone-500' : 'text-stone-500'">{{ t(kind === 'lut' ? 'grade.resourcesEmpty' : 'grade.resourcesPresetEmpty') }}</p>
        <div v-for="item in items" :key="item.id" class="mb-1 rounded px-2 py-1.5"
          :class="isDark ? 'hover:bg-white/5' : 'hover:bg-white/60'">
          <div class="flex items-center gap-2">
            <input v-if="renaming === item.hash" v-model="renameText" type="text"
              class="min-w-0 flex-1 rounded border bg-transparent px-1.5 py-0.5 text-[13px]"
              :class="isDark ? 'border-white/15 text-stone-100' : 'border-stone-900/15 text-stone-900'"
              @keydown.enter="saveRename(item)" />
            <span v-else class="min-w-0 flex-1 truncate text-[13px]" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ item.display_name }}</span>
            <span class="shrink-0 font-mono text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ shortHash(item.hash) }}</span>
            <button v-if="renaming === item.hash" type="button"
              class="inline-flex h-8 shrink-0 items-center gap-1 rounded-full px-2 text-xs font-medium"
              :class="isDark ? 'bg-amber-500/15 text-amber-200 hover:bg-amber-500/25' : 'bg-amber-100 text-amber-900 hover:bg-amber-200'"
              :disabled="busy"
              :title="t('grade.resourcesSave')"
              @click.stop="saveRename(item)">
              <IconCheck class="h-3.5 w-3.5" />
              {{ t('grade.resourcesSave') }}
            </button>
            <button v-else type="button"
              class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
              :disabled="busy"
              :title="t('grade.resourcesRename')"
              @click.stop="startRename(item)">
              <IconPencil class="h-3.5 w-3.5" />
            </button>
            <button type="button"
              class="inline-flex h-8 shrink-0 items-center gap-1 rounded-full px-2 text-xs font-medium transition"
              :disabled="busy || item.ref_count > 0"
              :class="item.ref_count > 0
                ? (isDark ? 'cursor-not-allowed bg-stone-800/40 text-stone-600' : 'cursor-not-allowed bg-stone-100 text-stone-300')
                : confirmHash === item.hash
                  ? 'bg-red-600 text-white hover:bg-red-500'
                  : (isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300')"
              :title="item.ref_count > 0 ? t('grade.resourcesRefs', { count: item.ref_count }) : (confirmHash === item.hash ? t('grade.resourcesConfirmDelete') : t('grade.resourcesDelete'))"
              @click.stop="remove(item)">
              <IconTrash class="h-3.5 w-3.5 shrink-0" />
              <span v-if="confirmHash === item.hash">{{ t('grade.resourcesConfirmDelete') }}</span>
            </button>
          </div>
          <p class="mt-0.5 text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-500'">
            {{ item.status === 'damaged' ? t('grade.resourcesDamaged') : t('grade.resourcesReady') }}
            <template v-if="item.lut_size"> · {{ item.lut_size }}³</template>
            <template v-if="item.ref_count"> · {{ t('grade.resourcesRefs', { count: item.ref_count }) }}</template>
          </p>
          <p v-if="item.ref_paths?.length" class="truncate text-[11px]" :class="isDark ? 'text-stone-600' : 'text-stone-400'">{{ item.ref_paths.join('  ') }}</p>
        </div>
      </div>
    </div>
    </div>
    </div>
  </AppDialog>
</template>
