<script setup>
import { computed, ref } from "vue";
import { t } from "../../i18n/index.js";
import { defaultWallpaper, wallpaperLayerState } from "../../utils/wallpaper.js";

const props = defineProps({
  busyLogin: { type: Boolean, required: true },
  loginForm: { type: Object, required: true },
  isDark: { type: Boolean, default: false },
});

defineEmits(["submit", "register"]);
const isRegisterMode = ref(false);
const wallpaperLayer = computed(() => wallpaperLayerState(defaultWallpaper(), props.isDark));
</script>

<template>
  <div class="mobile-login">
    <div class="pointer-events-none absolute inset-0" :style="wallpaperLayer.style"></div>
    <div class="relative z-10 flex min-h-0 flex-1 flex-col justify-end px-6 pb-10">
      <div class="mb-8">
        <p class="text-sm font-medium tracking-wide text-amber-600">{{ t("login.tagline") }}</p>
        <h1 class="mt-2 font-display text-4xl font-semibold" :class="isDark ? 'text-stone-50' : 'text-stone-900'">
          {{ t("login.title") }}
        </h1>
        <p class="mt-2 text-base" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
          {{ isRegisterMode ? t("login.createAccount") : t("login.appWelcome") }}
        </p>
      </div>

      <form class="grid gap-4" @submit.prevent="isRegisterMode ? $emit('register') : $emit('submit')">
        <label class="grid gap-2">
          <span class="text-sm font-medium" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ t("login.username") }}</span>
          <input
            v-model.trim="loginForm.username"
            autocomplete="username"
            class="mobile-login-input"
            :placeholder="isRegisterMode ? t('login.usernamePlaceholderRegister') : t('login.usernamePlaceholderLogin')"
          />
        </label>
        <label class="grid gap-2">
          <span class="text-sm font-medium" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ t("login.password") }}</span>
          <input
            v-model.trim="loginForm.passphrase"
            :autocomplete="isRegisterMode ? 'new-password' : 'current-password'"
            class="mobile-login-input"
            :placeholder="isRegisterMode ? t('login.setPassword') : t('login.enterPassword')"
            type="password"
          />
        </label>
        <label v-if="!isRegisterMode" class="flex items-center gap-3 text-sm" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
          <input v-model="loginForm.keepLogin" class="h-5 w-5 rounded border-stone-300 text-amber-700" type="checkbox" />
          <span>{{ t("login.keepLogin") }}</span>
        </label>
        <button class="mobile-login-submit" :disabled="busyLogin" type="submit">
          {{ busyLogin ? (isRegisterMode ? t("login.registering") : t("login.loggingIn")) : (isRegisterMode ? t("login.register") : t("login.login")) }}
        </button>
      </form>

      <p class="mt-5 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        <template v-if="isRegisterMode">
          {{ t("login.hasAccount") }}
          <button type="button" class="rounded-full px-2 py-1 font-medium text-amber-600" @click="isRegisterMode = false">{{ t("login.backToLogin") }}</button>
        </template>
        <template v-else>
          {{ t("login.noAccount") }}
          <button type="button" class="rounded-full px-2 py-1 font-medium text-amber-600" @click="isRegisterMode = true">{{ t("login.register") }}</button>
        </template>
      </p>
    </div>
  </div>
</template>
