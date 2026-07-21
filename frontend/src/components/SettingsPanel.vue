<script setup>
import { ref, watch, computed } from "vue";
import { availableLocales, t } from "../i18n/index.js";

const props = defineProps({
  isDark: { type: Boolean, default: false },
  settings: { type: Object, default: () => ({}) },
  aiStatus: { type: Object, default: null },
  members: { type: Array, default: () => ([]) },
  isAdmin: { type: Boolean, default: false },
  currentUser: { type: String, default: "" },
  userRole: { type: String, default: "" },
  currentLocale: { type: String, default: "en" },
  userAvatar: { type: String, default: null },
  onUpdatePassword: { type: Function, default: null },
  onUploadAvatar: { type: Function, default: null },
  onAiIndex: { type: Function, default: null },
  webdavShares: { type: Array, default: () => [] },
  onWebdavUpdate: { type: Function, default: null },
});

const emit = defineEmits(["update-settings", "scan-thumbnails", "update-preferences", "update-member", "update-password", "delete-member", "reset-member-password"]);

const defaultAiCpuThreads = Math.max(1, (typeof navigator !== "undefined" ? navigator.hardwareConcurrency || 4 : 4) - 1);

const localSettings = ref({});
const localPreferences = ref({ locale: "en" });
const localMembers = ref([]);
const passwordForm = ref({ current_password: "", new_password: "", confirm_password: "" });
const avatarInput = ref(null);
const avatarUploading = ref(false);
const aiIndexStarting = ref(false);

const webdavNewPath = ref("");
const webdavAdding = ref(false);

const activeSection = ref("preferences");
const mobileShowContent = ref(false);

const menuItems = computed(() => {
  const items = [
    { key: "preferences", icon: "lang" },
    { key: "account", icon: "user" },
    { key: "password", icon: "lock" },
  ];
  if (props.isAdmin) {
    items.push({ key: "members", icon: "users" });
    items.push({ key: "appSettings", icon: "cog" });
  }
  items.push({ key: "webdav", icon: "share" });
  return items;
});

function menuLabel(key) {
  const map = {
    preferences: "settings.preferences",
    account: "settings.accountInfo",
    password: "settings.password",
    members: "settings.memberManagement",
    appSettings: "settings.appSettings",
    webdav: "settings.webdavShares",
  };
  return t(map[key] || key);
}

function selectSection(key) {
  activeSection.value = key;
  mobileShowContent.value = true;
}

function goBackToMenu() {
  mobileShowContent.value = false;
}

async function addWebdavShare() {
  const path = webdavNewPath.value.trim().replace(/^\/+|\/+$/g, "");
  if (!path || !props.onWebdavUpdate) return;
  await props.onWebdavUpdate({ action: "add", path, readonly: true });
  webdavNewPath.value = "";
  webdavAdding.value = false;
}

async function removeWebdavShare(path) {
  if (!props.onWebdavUpdate) return;
  await props.onWebdavUpdate({ action: "remove", path });
}

async function toggleWebdavReadonly(share) {
  if (!props.onWebdavUpdate) return;
  await props.onWebdavUpdate({ action: "add", path: share.path, readonly: !share.readonly, anonymous: share.anonymous });
}

async function toggleWebdavAnonymous(share) {
  if (!share.anonymous && !confirm(t('settings.webdavAnonWarning'))) return;
  if (!props.onWebdavUpdate) return;
  await props.onWebdavUpdate({ action: "add", path: share.path, readonly: share.readonly, anonymous: !share.anonymous });
}

function webdavMountUrl(share) {
  const loc = window.location;
  const proto = loc.protocol;
  const host = loc.hostname || 'SERVER';
  const port = loc.port ? `:${loc.port}` : '';
  return `${proto}//${host}${port}/dav/${props.currentUser}/${share.path}`;
}

function webdavMountCmd(share) {
  return `mount -t davfs ${webdavMountUrl(share)} /mnt/share`;
}

function copyWebdavMountCmd(share) {
  navigator.clipboard.writeText(webdavMountCmd(share));
}

watch(
  () => props.settings,
  (val) => {
    localSettings.value = {
      thumbnail_generation_mode: "on_demand",
      thumbnail_scan_interval: "1d",
      ai_enabled: false,
      ai_provider: "local",
      ai_remote_url: "",
      ai_concurrency: 1,
      ai_batch_size: 64,
      ai_cpu_threads: defaultAiCpuThreads,
      ...val,
    };
  },
  { immediate: true, deep: true }
);

watch(
  () => props.currentLocale,
  (val) => {
    localPreferences.value = { locale: val || "en" };
  },
  { immediate: true }
);

watch(
  () => props.members,
  (val) => {
    localMembers.value = (val || []).map((member) => ({ ...member }));
  },
  { immediate: true, deep: true }
);

function save() {
  emit("update-settings", { ...localSettings.value });
}

function scanThumbnailsNow() {
  emit("scan-thumbnails");
}

function savePreferences() {
  emit("update-preferences", { ...localPreferences.value });
}

async function savePassword() {
  if (passwordForm.value.new_password !== passwordForm.value.confirm_password) {
    return;
  }

  const payload = {
    current_password: passwordForm.value.current_password,
    new_password: passwordForm.value.new_password,
  };

  let success = true;
  if (props.onUpdatePassword) {
    success = await props.onUpdatePassword(payload);
  } else {
    emit("update-password", payload);
  }

  if (success !== false) {
    passwordForm.value = { current_password: "", new_password: "", confirm_password: "" };
  }
}

function triggerAvatarUpload() {
  avatarInput.value?.click();
}

async function handleAvatarChange(event) {
  const file = event.target.files?.[0];
  if (!file || !props.onUploadAvatar) return;
  avatarUploading.value = true;
  try {
    await props.onUploadAvatar(file);
  } finally {
    avatarUploading.value = false;
    if (avatarInput.value) avatarInput.value.value = "";
  }
}

async function startAiIndex() {
  if (!props.onAiIndex || aiIndexStarting.value) return;
  aiIndexStarting.value = true;
  try {
    await props.onAiIndex();
  } finally {
    aiIndexStarting.value = false;
  }
}

function saveMember(member) {
  emit("update-member", {
    username: member.username,
    role: member.role,
    locale: member.locale,
  });
}

function confirmDeleteMember(member) {
  const msg = t("settings.confirmDeleteMember").replace("{username}", member.username);
  if (window.confirm(msg)) {
    emit("delete-member", member.username);
  }
}

function promptResetPassword(member) {
  const msg = t("settings.resetPasswordPrompt").replace("{username}", member.username);
  const newPwd = window.prompt(msg);
  if (newPwd && newPwd.trim()) {
    emit("reset-member-password", { username: member.username, new_password: newPwd.trim() });
  }
}
</script>

<template>
  <section class="flex h-full flex-col overflow-hidden rounded-2xl border shadow-sm"
    :class="isDark ? 'border-stone-800 bg-stone-900/90 text-stone-100' : 'border-stone-200 bg-white/90 text-stone-900'">

    <!-- Header with back button on mobile when viewing content -->
    <div class="flex items-center gap-2 border-b px-4 py-3 md:px-5 md:py-4"
      :class="isDark ? 'border-stone-700/60' : 'border-stone-200/60'">
      <button v-if="mobileShowContent" class="md:hidden mr-1 rounded-lg p-1 transition"
        :class="isDark ? 'hover:bg-stone-700' : 'hover:bg-stone-200'"
        @click="goBackToMenu">
        <svg class="h-4 w-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M15 19l-7-7 7-7"/></svg>
      </button>
      <h2 class="text-sm font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
        {{ mobileShowContent ? menuLabel(activeSection) : t('settings.title') }}
      </h2>
    </div>

    <div class="flex flex-1 overflow-hidden">
      <!-- Sidebar menu — always visible on md+, toggleable on mobile -->
      <nav class="flex-shrink-0 overflow-y-auto border-r md:w-48"
        :class="[
          isDark ? 'border-stone-700/60' : 'border-stone-200/60',
          mobileShowContent ? 'hidden md:block' : 'w-full md:w-48'
        ]">
        <ul class="flex flex-col py-2">
          <li v-for="item in menuItems" :key="item.key">
            <button class="flex w-full items-center gap-3 px-4 py-2.5 text-left text-sm transition"
              :class="[
                activeSection === item.key
                  ? (isDark ? 'bg-stone-800 text-stone-100 font-medium' : 'bg-stone-100 text-stone-900 font-medium')
                  : (isDark ? 'text-stone-400 hover:bg-stone-800/60 hover:text-stone-200' : 'text-stone-500 hover:bg-stone-50 hover:text-stone-800')
              ]"
              @click="selectSection(item.key)">
              <!-- icons -->
              <svg v-if="item.icon === 'lang'" class="h-4 w-4 flex-shrink-0" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="m10.5 21 5.25-11.25L21 21m-9-3h7.5M3 5.621a48.474 48.474 0 0 1 6-.371m0 0c1.12 0 2.233.038 3.334.114M9 5.25V3m3.334 2.364C11.176 10.658 7.69 15.08 3 17.502m9.334-12.138c.896.061 1.785.147 2.666.257m-4.589 8.495A18.023 18.023 0 0 1 3.413 7.18"/></svg>
              <svg v-else-if="item.icon === 'user'" class="h-4 w-4 flex-shrink-0" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M15.75 6a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0ZM4.501 20.118a7.5 7.5 0 0 1 14.998 0A17.933 17.933 0 0 1 12 21.75c-2.676 0-5.216-.584-7.499-1.632Z"/></svg>
              <svg v-else-if="item.icon === 'lock'" class="h-4 w-4 flex-shrink-0" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M16.5 10.5V6.75a4.5 4.5 0 1 0-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 0 0 2.25-2.25v-6.75a2.25 2.25 0 0 0-2.25-2.25H6.75a2.25 2.25 0 0 0-2.25 2.25v6.75a2.25 2.25 0 0 0 2.25 2.25Z"/></svg>
              <svg v-else-if="item.icon === 'users'" class="h-4 w-4 flex-shrink-0" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M15 19.128a9.38 9.38 0 0 0 2.625.372 9.337 9.337 0 0 0 4.121-.952 4.125 4.125 0 0 0-7.533-2.493M15 19.128v-.003c0-1.113-.285-2.16-.786-3.07M15 19.128v.106A12.318 12.318 0 0 1 8.624 21c-2.331 0-4.512-.645-6.374-1.766l-.001-.109a6.375 6.375 0 0 1 11.964-3.07M12 6.375a3.375 3.375 0 1 1-6.75 0 3.375 3.375 0 0 1 6.75 0Zm8.25 2.25a2.625 2.625 0 1 1-5.25 0 2.625 2.625 0 0 1 5.25 0Z"/></svg>
              <svg v-else-if="item.icon === 'cog'" class="h-4 w-4 flex-shrink-0" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.325.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 0 1 1.37.49l1.296 2.247a1.125 1.125 0 0 1-.26 1.431l-1.003.827c-.293.241-.438.613-.43.992a7.723 7.723 0 0 1 0 .255c-.008.378.137.75.43.991l1.004.827c.424.35.534.955.26 1.43l-1.298 2.247a1.125 1.125 0 0 1-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.47 6.47 0 0 1-.22.128c-.331.183-.581.495-.644.869l-.213 1.281c-.09.543-.56.94-1.11.94h-2.594c-.55 0-1.019-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 0 1-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 0 1-1.369-.49l-1.297-2.247a1.125 1.125 0 0 1 .26-1.431l1.004-.827c.292-.24.437-.613.43-.991a6.932 6.932 0 0 1 0-.255c.007-.38-.138-.751-.43-.992l-1.004-.827a1.125 1.125 0 0 1-.26-1.43l1.297-2.247a1.125 1.125 0 0 1 1.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.086.22-.128.332-.183.582-.495.644-.869l.214-1.28Z"/><path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z"/></svg>
              <svg v-else-if="item.icon === 'share'" class="h-4 w-4 flex-shrink-0" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M7.217 10.907a2.25 2.25 0 1 0 0 2.186m0-2.186c.18.324.283.696.283 1.093s-.103.77-.283 1.093m0-2.186 9.566-5.314m-9.566 7.5 9.566 5.314m0 0a2.25 2.25 0 1 0 3.935 2.186 2.25 2.25 0 0 0-3.935-2.186Zm0-12.814a2.25 2.25 0 1 0 3.933-2.185 2.25 2.25 0 0 0-3.933 2.185Z"/></svg>
              <span class="truncate">{{ menuLabel(item.key) }}</span>
              <svg class="ml-auto h-4 w-4 flex-shrink-0 md:hidden" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="m9 5 7 7-7 7"/></svg>
            </button>
          </li>
        </ul>
      </nav>

      <!-- Content area — always visible on md+, toggleable on mobile -->
      <div class="flex-1 overflow-y-auto app-scrollbar"
        :class="mobileShowContent ? '' : 'hidden md:block'">
        <div class="p-4 md:p-5">
          <Transition name="settings-section" mode="out-in">
          <div :key="activeSection">

          <!-- Preferences -->
          <div v-if="activeSection === 'preferences'">
            <div class="flex flex-col gap-4 rounded-xl border p-4"
              :class="isDark ? 'border-stone-700 bg-stone-800/60' : 'border-stone-200 bg-stone-50'">
              <div>
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.language') }}</label>
                <select v-model="localPreferences.locale"
                  class="w-full rounded-lg border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'">
                  <option v-for="lang in availableLocales" :key="lang" :value="lang">
                    {{ lang === 'zh' ? t('settings.langChinese') : t('settings.langEnglish') }}
                  </option>
                </select>
              </div>
              <button class="self-start rounded-xl px-5 py-2 text-sm font-medium transition"
                :class="isDark ? 'bg-stone-100 text-stone-900 hover:bg-white' : 'bg-stone-900 text-white hover:bg-stone-800'"
                @click="savePreferences">
                {{ t('settings.savePreferences') }}
              </button>
            </div>
          </div>

          <!-- Account -->
          <div v-else-if="activeSection === 'account'">
            <div class="rounded-xl border p-4"
              :class="isDark ? 'border-stone-700 bg-stone-800/60' : 'border-stone-200 bg-stone-50'">
              <div class="flex items-start gap-4">
                <div class="flex flex-col items-center gap-2">
                  <button
                    class="group relative h-16 w-16 flex-shrink-0 overflow-hidden rounded-full border-2 transition focus:outline-none"
                    :class="isDark ? 'border-stone-600 hover:border-stone-400' : 'border-stone-300 hover:border-stone-500'"
                    :title="t('settings.changeAvatar')"
                    :disabled="avatarUploading"
                    @click="triggerAvatarUpload">
                    <img v-if="userAvatar" :src="`/api/avatar/${userAvatar}`" alt="avatar"
                      class="h-full w-full object-cover" />
                    <span v-else
                      class="flex h-full w-full items-center justify-center text-xl font-bold uppercase"
                      :class="isDark ? 'bg-stone-700 text-stone-300' : 'bg-stone-200 text-stone-600'">
                      {{ currentUser?.charAt(0) || '?' }}
                    </span>
                    <span
                      class="absolute inset-0 flex items-center justify-center bg-black/40 text-xs font-medium text-white opacity-0 transition group-hover:opacity-100">
                      <svg class="h-4 w-4" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M3 9a2 2 0 012-2h.93a2 2 0 001.664-.89l.812-1.22A2 2 0 0110.07 4h3.86a2 2 0 011.664.89l.812 1.22A2 2 0 0018.07 7H19a2 2 0 012 2v9a2 2 0 01-2 2H5a2 2 0 01-2-2V9z"/><circle cx="12" cy="13" r="3"/></svg>
                    </span>
                  </button>
                  <input ref="avatarInput" type="file" accept="image/*" class="hidden" @change="handleAvatarChange" />
                  <span class="text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t('settings.avatarHint') }}</span>
                </div>
                <div class="flex-1 grid grid-cols-2 gap-3 text-sm">
                  <div>
                    <span class="text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('settings.username') }}</span>
                    <p class="font-medium">{{ currentUser }}</p>
                  </div>
                  <div>
                    <span class="text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('settings.role') }}</span>
                    <p class="font-medium">
                      <span v-if="userRole === 'admin'"
                        class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
                        :class="isDark ? 'bg-amber-900/50 text-amber-200' : 'bg-amber-100 text-amber-800'">{{ t('settings.admin') }}</span>
                      <span v-else
                        class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
                        :class="isDark ? 'bg-stone-700 text-stone-300' : 'bg-stone-200 text-stone-600'">{{ t('settings.user') }}</span>
                    </p>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <!-- Password -->
          <div v-else-if="activeSection === 'password'">
            <div class="flex flex-col gap-4 rounded-xl border p-4"
              :class="isDark ? 'border-stone-700 bg-stone-800/60' : 'border-stone-200 bg-stone-50'">
              <div>
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.currentPassword') }}</label>
                <input v-model="passwordForm.current_password" type="password"
                  class="w-full rounded-lg border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'" />
              </div>
              <div>
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.newPassword') }}</label>
                <input v-model="passwordForm.new_password" type="password"
                  class="w-full rounded-lg border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'" />
              </div>
              <div>
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.confirmPassword') }}</label>
                <input v-model="passwordForm.confirm_password" type="password"
                  class="w-full rounded-lg border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'" />
              </div>
              <p v-if="passwordForm.confirm_password && passwordForm.new_password !== passwordForm.confirm_password"
                class="text-sm text-rose-500">
                {{ t('settings.passwordMismatch') }}
              </p>
              <button class="self-start rounded-xl px-5 py-2 text-sm font-medium transition"
                :disabled="!passwordForm.current_password || !passwordForm.new_password || passwordForm.new_password !== passwordForm.confirm_password"
                :class="isDark ? 'bg-stone-100 text-stone-900 hover:bg-white disabled:opacity-60' : 'bg-stone-900 text-white hover:bg-stone-800 disabled:opacity-60'"
                @click="savePassword">
                {{ t('settings.savePassword') }}
              </button>
            </div>
          </div>

          <!-- Members (admin) -->
          <div v-else-if="activeSection === 'members' && isAdmin">
            <div class="flex flex-col gap-3 rounded-xl border p-4"
              :class="isDark ? 'border-stone-700 bg-stone-800/60' : 'border-stone-200 bg-stone-50'">
              <div v-if="localMembers.length === 0" class="text-sm"
                :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                {{ t('settings.noMembers') }}
              </div>
              <div v-for="member in localMembers" :key="member.username"
                class="rounded-xl border p-4"
                :class="isDark ? 'border-stone-700/80 bg-stone-900/50' : 'border-stone-200 bg-white/80'">
                <div class="mb-3 flex items-center justify-between gap-3">
                  <div>
                    <p class="text-sm font-semibold">{{ member.username }}</p>
                    <p class="text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                      {{ member.username === currentUser ? t('settings.currentUserLabel') : t('settings.memberLabel') }}
                    </p>
                  </div>
                  <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
                    :class="member.role === 'admin'
                      ? (isDark ? 'bg-amber-900/50 text-amber-200' : 'bg-amber-100 text-amber-800')
                      : (isDark ? 'bg-stone-700 text-stone-300' : 'bg-stone-200 text-stone-600')">
                    {{ member.role === 'admin' ? t('settings.admin') : t('settings.user') }}
                  </span>
                </div>
                <div class="grid gap-3 md:grid-cols-2">
                  <div>
                    <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.role') }}</label>
                    <select v-model="member.role"
                      :disabled="member.username === currentUser"
                      class="w-full rounded-lg border px-3 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-60"
                      :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'">
                      <option value="admin">{{ t('settings.admin') }}</option>
                      <option value="user">{{ t('settings.user') }}</option>
                    </select>
                  </div>
                  <div>
                    <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.language') }}</label>
                    <select v-model="member.locale"
                      :disabled="member.username === currentUser"
                      class="w-full rounded-lg border px-3 py-2 text-sm"
                      :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'">
                      <option v-for="lang in availableLocales" :key="lang" :value="lang">
                        {{ lang === 'zh' ? t('settings.langChinese') : t('settings.langEnglish') }}
                      </option>
                    </select>
                  </div>
                </div>
                <div class="mt-3 flex flex-wrap items-center justify-between gap-3">
                  <p class="text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                    {{ member.username === currentUser ? t('settings.currentUserRoleLocked') : t('settings.memberEditable') }}
                  </p>
                  <div class="flex items-center gap-2">
                    <template v-if="member.username !== currentUser">
                      <button class="rounded-xl px-3 py-2 text-sm font-medium transition"
                        :class="isDark ? 'bg-stone-700 text-stone-200 hover:bg-stone-600' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
                        @click="promptResetPassword(member)">
                        {{ t('settings.resetPassword') }}
                      </button>
                      <button class="rounded-xl px-3 py-2 text-sm font-medium transition"
                        :class="isDark ? 'bg-rose-900/60 text-rose-200 hover:bg-rose-800/70' : 'bg-rose-100 text-rose-700 hover:bg-rose-200'"
                        @click="confirmDeleteMember(member)">
                        {{ t('settings.deleteMember') }}
                      </button>
                    </template>
                    <button class="rounded-xl px-4 py-2 text-sm font-medium transition"
                      :disabled="member.username === currentUser"
                      :class="isDark ? 'bg-stone-100 text-stone-900 hover:bg-white' : 'bg-stone-900 text-white hover:bg-stone-800'"
                      @click="saveMember(member)">
                      {{ t('settings.saveMember') }}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <!-- App Settings (admin) -->
          <div v-else-if="activeSection === 'appSettings' && isAdmin">
            <div class="flex flex-col gap-4 rounded-xl border p-4"
              :class="isDark ? 'border-stone-700 bg-stone-800/60' : 'border-stone-200 bg-stone-50'">
              <div class="rounded-xl border p-4"
                :class="isDark ? 'border-stone-700/80 bg-stone-900/50' : 'border-stone-200 bg-white/80'">
                <div class="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <p class="text-sm font-semibold">{{ t('settings.aiTitle') }}</p>
                    <p class="mt-1 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                      {{ t('settings.aiSubtitle') }}
                    </p>
                  </div>
                  <span class="inline-flex items-center rounded-full px-2.5 py-1 text-xs font-medium"
                    :class="props.aiStatus?.enabled
                      ? (isDark ? 'bg-emerald-900/60 text-emerald-200' : 'bg-emerald-100 text-emerald-700')
                      : (isDark ? 'bg-stone-700 text-stone-300' : 'bg-stone-200 text-stone-600')">
                    {{ props.aiStatus?.enabled ? t('settings.aiEnabled') : t('settings.aiDisabled') }}
                  </span>
                </div>
                <div class="mt-4 grid gap-3 md:grid-cols-2">
                  <div>
                    <span class="block text-[11px] font-medium uppercase tracking-wide" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('settings.aiProvider') }}</span>
                    <p class="mt-1 text-sm">{{ props.aiStatus?.provider?.kind || '-' }}</p>
                  </div>
                  <div>
                    <span class="block text-[11px] font-medium uppercase tracking-wide" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('settings.aiBackend') }}</span>
                    <p class="mt-1 text-sm break-all">{{ props.aiStatus?.provider?.backend || '-' }}</p>
                  </div>
                  <div>
                    <span class="block text-[11px] font-medium uppercase tracking-wide" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('settings.aiHealth') }}</span>
                    <p class="mt-1 text-sm">{{ props.aiStatus?.provider?.healthy ? t('settings.aiHealthy') : t('settings.aiUnavailable') }}</p>
                  </div>
                  <div>
                    <span class="block text-[11px] font-medium uppercase tracking-wide" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('settings.aiGeneration') }}</span>
                    <p class="mt-1 text-sm">{{ props.aiStatus?.owner_index?.current_generation || '-' }}</p>
                  </div>
                  <div>
                    <span class="block text-[11px] font-medium uppercase tracking-wide" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('settings.aiConcurrency') }}</span>
                    <p class="mt-1 text-sm">{{ props.aiStatus?.limits?.concurrency ?? '-' }}</p>
                  </div>
                  <div>
                    <span class="block text-[11px] font-medium uppercase tracking-wide" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('settings.aiIndexState') }}</span>
                    <p class="mt-1 text-sm">{{ props.aiStatus?.owner_index?.state || '-' }}</p>
                  </div>
                </div>
                <div class="mt-4 grid gap-2 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                  <div class="flex flex-wrap items-center gap-3 pb-1">
                    <button
                      class="rounded-lg px-3 py-1.5 text-xs font-medium transition"
                      :disabled="aiIndexStarting"
                      :class="isDark ? 'bg-sky-700 text-white hover:bg-sky-600 disabled:opacity-60' : 'bg-sky-500 text-white hover:bg-sky-600 disabled:opacity-60'"
                      @click="startAiIndex">
                      {{ aiIndexStarting ? t('settings.aiStartingIndex') : t('settings.aiStartIndex') }}
                    </button>
                    <span>{{ t('settings.aiStartIndexHint') }}</span>
                  </div>
                  <p>{{ t('settings.aiCapabilityImage') }}: {{ props.aiStatus?.capabilities?.image_search ? t('settings.aiReady') : t('settings.aiPending') }}</p>
                  <p>{{ t('settings.aiCapabilityText') }}: {{ props.aiStatus?.capabilities?.text_search ? t('settings.aiReady') : t('settings.aiPending') }}</p>
                  <p>{{ t('settings.aiCapabilityDuplicates') }}: {{ props.aiStatus?.capabilities?.duplicates ? t('settings.aiReady') : t('settings.aiPending') }}</p>
                  <p>{{ t('settings.aiCapabilityFaces') }}: {{ props.aiStatus?.capabilities?.faces ? t('settings.aiReady') : t('settings.aiPending') }}</p>
                  <p>{{ t('settings.aiIndexRoot') }}: {{ props.aiStatus?.owner_index?.root_exists ? t('settings.aiReady') : t('settings.aiPending') }}</p>
                  <p>{{ t('settings.aiIndexManifest') }}: {{ props.aiStatus?.owner_index?.manifest_present ? t('settings.aiReady') : t('settings.aiPending') }}</p>
                  <p>{{ t('settings.aiPhotosMeta') }}: {{ props.aiStatus?.owner_index?.photos_meta_present ? t('settings.aiReady') : t('settings.aiPending') }}</p>
                  <p>{{ t('settings.aiHashesBin') }}: {{ props.aiStatus?.owner_index?.hashes_bin_present ? t('settings.aiReady') : t('settings.aiPending') }}</p>
                  <p>{{ t('settings.aiAlbumsTotal') }}: {{ props.aiStatus?.owner_index?.stats?.albums_total ?? '-' }}</p>
                  <p>{{ t('settings.aiPhotosTotal') }}: {{ props.aiStatus?.owner_index?.stats?.photos_total ?? '-' }}</p>
                  <p>{{ t('settings.aiPhotosIndexed') }}: {{ props.aiStatus?.owner_index?.stats?.photos_indexed ?? '-' }}</p>
                  <p>{{ t('settings.aiPhotosReused') }}: {{ props.aiStatus?.owner_index?.stats?.photos_reused ?? '-' }}</p>
                  <p>{{ t('settings.aiImageEmbeddingsPending') }}: {{ props.aiStatus?.owner_index?.stats?.image_embeddings_pending ?? '-' }}</p>
                  <p>{{ t('settings.aiDuplicateHashesPending') }}: {{ props.aiStatus?.owner_index?.stats?.duplicate_hashes_pending ?? '-' }}</p>
                  <p>{{ t('settings.aiFaceEmbeddingsPending') }}: {{ props.aiStatus?.owner_index?.stats?.face_embeddings_pending ?? '-' }}</p>
                  <p>{{ t('settings.aiSourcesTotal') }}: {{ props.aiStatus?.owner_index?.sources_total ?? '-' }}</p>
                  <p v-if="props.aiStatus?.provider?.reason">{{ t('settings.aiReason') }}: {{ props.aiStatus.provider.reason }}</p>
                  <p v-if="props.aiStatus?.provider?.model_root">{{ t('settings.aiModelRoot') }}: {{ props.aiStatus.provider.model_root }}</p>
                  <p v-if="props.aiStatus?.provider?.remote_token_configured !== undefined">
                    {{ t('settings.aiRemoteTokenConfigured') }}:
                    {{ props.aiStatus.provider.remote_token_configured ? t('settings.aiRemoteTokenYes') : t('settings.aiRemoteTokenNo') }}
                  </p>
                </div>
              </div>
              <div class="rounded-xl border p-4"
                :class="isDark ? 'border-stone-700/80 bg-stone-900/50' : 'border-stone-200 bg-white/80'">
                <p class="text-sm font-semibold">{{ t('settings.aiControlsTitle') }}</p>
                <p class="mt-1 mb-4 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('settings.aiControlsHint') }}</p>
                <div class="grid gap-4 md:grid-cols-2">
                  <div class="flex items-center gap-3">
                    <label class="relative inline-flex cursor-pointer items-center">
                      <input type="checkbox" v-model="localSettings.ai_enabled" class="peer sr-only" />
                      <div class="peer h-5 w-9 rounded-full after:absolute after:left-[2px] after:top-[2px] after:h-4 after:w-4 after:rounded-full after:transition-all peer-checked:after:translate-x-full"
                        :class="isDark
                          ? 'bg-stone-700 after:bg-stone-400 peer-checked:bg-amber-600 peer-checked:after:bg-white'
                          : 'bg-stone-300 after:bg-white peer-checked:bg-amber-500 peer-checked:after:bg-white'"></div>
                    </label>
                    <span class="text-sm" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ t('settings.aiEnabledToggle') }}</span>
                  </div>
                  <div>
                    <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.aiProviderSelect') }}</label>
                    <select v-model="localSettings.ai_provider"
                      class="w-full rounded-lg border px-3 py-2 text-sm"
                      :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'">
                      <option value="local">{{ t('settings.aiProviderLocal') }}</option>
                      <option value="remote">{{ t('settings.aiProviderRemote') }}</option>
                    </select>
                  </div>
                  <div v-if="localSettings.ai_provider === 'remote'" class="md:col-span-2">
                    <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.aiRemoteUrl') }}</label>
                    <input v-model="localSettings.ai_remote_url" type="url" placeholder="https://ai.example.com"
                      class="w-full rounded-lg border px-3 py-2 text-sm"
                      :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
                  </div>
                  <div>
                    <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.aiConcurrency') }}</label>
                    <input v-model.number="localSettings.ai_concurrency" type="number" min="1" max="16" placeholder="1"
                      class="w-full rounded-lg border px-3 py-2 text-sm"
                      :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
                  </div>
                  <div>
                    <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.aiBatchSize') }}</label>
                    <input v-model.number="localSettings.ai_batch_size" type="number" min="1" max="64" placeholder="64"
                      class="w-full rounded-lg border px-3 py-2 text-sm"
                      :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
                    <p class="mt-1 text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t('settings.aiBatchSizeHint') }}</p>
                  </div>
                  <div>
                    <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.aiCpuThreads') }}</label>
                    <input v-model.number="localSettings.ai_cpu_threads" type="number" min="1" max="64"
                      :placeholder="String(defaultAiCpuThreads)"
                      class="w-full rounded-lg border px-3 py-2 text-sm"
                      :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
                    <p class="mt-1 text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t('settings.aiCpuThreadsHint') }}</p>
                  </div>
                </div>
              </div>
              <div>
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.siteName') }}</label>
                <input v-model="localSettings.site_name" type="text" placeholder="Album"
                  class="w-full rounded-lg border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
              </div>
              <div>
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.thumbnailQuality') }}</label>
                <input v-model.number="localSettings.thumbnail_quality" type="number" min="1" max="100" placeholder="85"
                  class="w-full rounded-lg border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
              </div>
              <div>
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.thumbnailGenerationMode') }}</label>
                <select v-model="localSettings.thumbnail_generation_mode"
                  class="w-full rounded-lg border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'">
                  <option value="on_demand">{{ t('settings.thumbnailGenerationOnDemand') }}</option>
                  <option value="background">{{ t('settings.thumbnailGenerationBackground') }}</option>
                </select>
                <p class="mt-1 text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t('settings.thumbnailGenerationModeHint') }}</p>
              </div>
              <div v-if="localSettings.thumbnail_generation_mode === 'background'">
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.thumbnailScanInterval') }}</label>
                <select v-model="localSettings.thumbnail_scan_interval"
                  class="w-full rounded-lg border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'">
                  <option value="12h">{{ t('settings.thumbnailScanEvery12Hours') }}</option>
                  <option value="1d">{{ t('settings.thumbnailScanDaily') }}</option>
                  <option value="3d">{{ t('settings.thumbnailScanEvery3Days') }}</option>
                  <option value="7d">{{ t('settings.thumbnailScanWeekly') }}</option>
                </select>
                <p class="mt-1 text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t('settings.thumbnailScanIntervalHint') }}</p>
              </div>
              <button type="button"
                class="self-start rounded-lg px-4 py-2 text-sm font-medium transition"
                :class="isDark ? 'bg-stone-700 text-stone-100 hover:bg-stone-600' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
                @click="scanThumbnailsNow">
                {{ t('settings.thumbnailScanNow') }}
              </button>
              <div>
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.thumbnailConcurrency') }}</label>
                <input v-model.number="localSettings.thumbnail_concurrency" type="number" min="1" max="32" placeholder="3"
                  class="w-full rounded-lg border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
                <p class="mt-1 text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t('settings.thumbnailConcurrencyHint') }}</p>
              </div>
              <div>
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.darktableConcurrency') }}</label>
                <input v-model.number="localSettings.darktable_concurrency" type="number" min="1" max="32" placeholder="1"
                  class="w-full rounded-lg border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
                <p class="mt-1 text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t('settings.darktableConcurrencyHint') }}</p>
              </div>
              <div>
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.photosPerPage') }}</label>
                <input v-model.number="localSettings.photos_per_page" type="number" min="10" max="500" placeholder="100"
                  class="w-full rounded-lg border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
              </div>
              <div class="flex items-center gap-3">
                <label class="relative inline-flex cursor-pointer items-center">
                  <input type="checkbox" v-model="localSettings.allow_registration" class="peer sr-only" />
                  <div class="peer h-5 w-9 rounded-full after:absolute after:left-[2px] after:top-[2px] after:h-4 after:w-4 after:rounded-full after:transition-all peer-checked:after:translate-x-full"
                    :class="isDark
                      ? 'bg-stone-700 after:bg-stone-400 peer-checked:bg-sky-600 peer-checked:after:bg-white'
                      : 'bg-stone-300 after:bg-white peer-checked:bg-sky-500 peer-checked:after:bg-white'"></div>
                </label>
                <span class="text-sm" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ t('settings.allowRegistration') }}</span>
              </div>
              <div>
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('settings.cacheRetentionDays') }}</label>
                <input v-model.number="localSettings.cache_retention_days" type="number" min="0" max="3650" placeholder="0"
                  class="w-full rounded-lg border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
                <p class="mt-1 text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t('settings.cacheRetentionHint') }}</p>
              </div>
              <button class="mt-2 self-start rounded-xl px-5 py-2 text-sm font-medium transition"
                :class="isDark ? 'bg-stone-100 text-stone-900 hover:bg-white' : 'bg-stone-900 text-white hover:bg-stone-800'"
                @click="save">
                {{ t('settings.save') }}
              </button>
            </div>
          </div>

          <!-- WebDAV Shares -->
          <div v-else-if="activeSection === 'webdav'">
            <div class="flex flex-col gap-2 rounded-xl border p-4"
              :class="isDark ? 'border-stone-700 bg-stone-800/60' : 'border-stone-200 bg-stone-50'">
              <p class="text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('settings.webdavHint') }}</p>
              <p class="text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t('settings.webdavCredentialHint') }}</p>

              <div v-if="webdavShares.length === 0" class="text-sm py-2"
                :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                {{ t('settings.webdavNoShares') }}
              </div>
              <div v-for="share in webdavShares" :key="share.path"
                class="flex flex-col md:flex-row md:items-center justify-between gap-2 md:gap-3 rounded-lg border px-3 py-2"
                :class="isDark ? 'border-stone-700 bg-stone-900/50' : 'border-stone-200 bg-white'">
                <div class="flex items-center gap-2 min-w-0">
                  <svg class="h-4 w-4 flex-shrink-0" :class="isDark ? 'text-stone-400' : 'text-stone-500'" viewBox="0 0 20 20" fill="currentColor">
                    <path d="M2 6a2 2 0 012-2h5l2 2h5a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z" />
                  </svg>
                  <span class="text-sm truncate">{{ share.path }}</span>
                </div>
                <div class="flex flex-wrap items-center gap-3 flex-shrink-0">
                  <label class="flex items-center gap-1.5 cursor-pointer">
                    <span class="text-[10px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('settings.webdavReadonly') }}</span>
                    <label class="relative inline-flex cursor-pointer items-center">
                      <input type="checkbox" :checked="share.readonly" @change="toggleWebdavReadonly(share)" class="peer sr-only" />
                      <div class="peer h-4 w-7 rounded-full after:absolute after:left-[2px] after:top-[2px] after:h-3 after:w-3 after:rounded-full after:transition-all peer-checked:after:translate-x-3"
                        :class="isDark
                          ? 'bg-stone-700 after:bg-stone-400 peer-checked:bg-sky-600 peer-checked:after:bg-white'
                          : 'bg-stone-300 after:bg-white peer-checked:bg-sky-500 peer-checked:after:bg-white'"></div>
                    </label>
                  </label>
                  <label class="flex items-center gap-1.5 cursor-pointer">
                    <span class="text-[10px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('settings.webdavAnonymous') }}</span>
                    <label class="relative inline-flex cursor-pointer items-center">
                      <input type="checkbox" :checked="share.anonymous" @change="toggleWebdavAnonymous(share)" class="peer sr-only" />
                      <div class="peer h-4 w-7 rounded-full after:absolute after:left-[2px] after:top-[2px] after:h-3 after:w-3 after:rounded-full after:transition-all peer-checked:after:translate-x-3"
                        :class="isDark
                          ? 'bg-stone-700 after:bg-stone-400 peer-checked:bg-sky-600 peer-checked:after:bg-white'
                          : 'bg-stone-300 after:bg-white peer-checked:bg-sky-500 peer-checked:after:bg-white'"></div>
                    </label>
                  </label>
                  <button
                    class="rounded-lg px-2 py-1 text-xs font-medium transition"
                    :class="isDark ? 'bg-stone-700 text-stone-300 hover:bg-stone-600' : 'bg-stone-200 text-stone-600 hover:bg-stone-300'"
                    :title="webdavMountCmd(share)"
                    @click="copyWebdavMountCmd(share)">
                    <svg class="h-3.5 w-3.5 inline-block" viewBox="0 0 20 20" fill="currentColor">
                      <path d="M8 2a1 1 0 000 2h2a1 1 0 100-2H8z" />
                      <path d="M3 5a2 2 0 012-2 3 3 0 003 3h2a3 3 0 003-3 2 2 0 012 2v6h-4.586l1.293-1.293a1 1 0 00-1.414-1.414l-3 3a1 1 0 000 1.414l3 3a1 1 0 001.414-1.414L10.414 13H15v3a2 2 0 01-2 2H5a2 2 0 01-2-2V5zM15 11h2a1 1 0 110 2h-2v-2z" />
                    </svg>
                    {{ t('settings.webdavCopyMount') }}
                  </button>
                  <button
                    class="rounded-lg px-2 py-1 text-xs font-medium transition"
                    :class="isDark ? 'bg-rose-900/60 text-rose-200 hover:bg-rose-800' : 'bg-rose-100 text-rose-700 hover:bg-rose-200'"
                    @click="removeWebdavShare(share.path)">
                    {{ t('settings.webdavRemove') }}
                  </button>
                </div>
              </div>

              <div v-if="webdavAdding" class="flex items-center gap-2 mt-1">
                <input v-model="webdavNewPath" type="text" :placeholder="t('settings.webdavPathPlaceholder')"
                  class="flex-1 rounded-lg border px-3 py-1.5 text-sm outline-none"
                  :class="isDark ? 'border-stone-600 bg-stone-900 text-stone-100 placeholder-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder-stone-400'"
                  @keyup.enter="addWebdavShare" />
                <button class="rounded-lg px-3 py-1.5 text-xs font-medium transition"
                  :class="isDark ? 'bg-sky-700 text-white hover:bg-sky-600' : 'bg-sky-500 text-white hover:bg-sky-600'"
                  @click="addWebdavShare">{{ t('settings.webdavConfirm') }}</button>
                <button class="rounded-lg px-3 py-1.5 text-xs font-medium transition"
                  :class="isDark ? 'bg-stone-700 text-stone-300 hover:bg-stone-600' : 'bg-stone-200 text-stone-600 hover:bg-stone-300'"
                  @click="webdavAdding = false; webdavNewPath = ''">{{ t('settings.webdavCancel') }}</button>
              </div>
              <button v-else class="self-start mt-1 rounded-lg px-3 py-1.5 text-xs font-medium transition"
                :class="isDark ? 'bg-emerald-900/60 text-emerald-200 hover:bg-emerald-800' : 'bg-emerald-100 text-emerald-700 hover:bg-emerald-200'"
                @click="webdavAdding = true">
                + {{ t('settings.webdavAdd') }}
              </button>
            </div>
          </div>

          </div>
          </Transition>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
@media (max-width: 767px) {
  .settings-section-enter-active,
  .settings-section-leave-active {
    transition: opacity 180ms ease, transform 180ms ease;
  }

  .settings-section-enter-from {
    opacity: 0;
    transform: translateX(14px);
  }

  .settings-section-leave-to {
    opacity: 0;
    transform: translateX(-8px);
  }
}

@media (prefers-reduced-motion: reduce) {
  .settings-section-enter-active,
  .settings-section-leave-active {
    transition: none;
  }
}
</style>
