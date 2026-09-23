<script setup>
import { computed, ref } from "vue";
import { t } from "../i18n/index.js";
import { defaultWallpaper, wallpaperLayerState } from "../utils/wallpaper.js";

const props = defineProps({
  busyLogin: {
    type: Boolean,
    required: true,
  },
  loginForm: {
    type: Object,
    required: true,
  },
  isDark: {
    type: Boolean,
    default: false,
  },
});

defineEmits(["submit", "register"]);
const isRegisterMode = ref(false);
const wallpaperLayer = computed(() => wallpaperLayerState(defaultWallpaper(), props.isDark));
</script>

<template>
  <div class="fixed inset-0 flex items-center justify-center p-4">
    <div class="pointer-events-none absolute inset-0" :style="wallpaperLayer.style"></div>
    <div
      v-if="wallpaperLayer.showGrid"
      class="pointer-events-none absolute inset-0 opacity-25 [background-image:linear-gradient(rgba(125,98,73,0.06)_1px,transparent_1px),linear-gradient(90deg,rgba(125,98,73,0.06)_1px,transparent_1px)] [background-size:28px_28px] [mask-image:linear-gradient(to_bottom,rgba(0,0,0,0.75),transparent_88%)]"
    ></div>

    <section
      class="app-frost relative z-10 w-full max-w-md rounded-[2rem] border p-8 shadow-[0_22px_60px_rgba(71,43,17,0.16)]"
      :class="isDark ? 'border-white/10 text-stone-100' : 'border-white/70 text-stone-900'"
    >
      <div class="mb-6 text-center">
        <p class="mb-2 text-xs font-semibold uppercase tracking-[0.24em] text-amber-700">
          Rust + Axum + Vue
        </p>
        <h1
          class="text-3xl font-semibold font-display"
          :class="isDark ? 'text-stone-100' : 'text-stone-900'"
        >
          {{ t('login.title') }}
        </h1>
        <p class="mt-2 text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ isRegisterMode ? t('login.createAccount') : t('login.loginDescription') }}</p>
      </div>

      <form class="grid gap-4" @submit.prevent="isRegisterMode ? $emit('register') : $emit('submit')">
        <label class="grid gap-2">
          <span class="text-sm" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('login.username') }}</span>
          <input
            v-model.trim="loginForm.username"
            autocomplete="username"
            class="rounded-2xl border px-4 py-3 outline-none transition"
            :class="isDark ? 'border-stone-700 bg-stone-900/60 text-stone-100 focus:border-amber-500 focus:ring-4 focus:ring-amber-900/40' : 'border-stone-200 bg-white/85 text-stone-900 focus:border-amber-500 focus:ring-4 focus:ring-amber-100'"
            :placeholder="isRegisterMode ? t('login.usernamePlaceholderRegister') : t('login.usernamePlaceholderLogin')"
          />
        </label>

        <label class="grid gap-2">
          <span class="text-sm" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('login.password') }}</span>
          <input
            v-model.trim="loginForm.passphrase"
            :autocomplete="isRegisterMode ? 'new-password' : 'current-password'"
            class="rounded-2xl border px-4 py-3 outline-none transition"
            :class="isDark ? 'border-stone-700 bg-stone-900/60 text-stone-100 focus:border-amber-500 focus:ring-4 focus:ring-amber-900/40' : 'border-stone-200 bg-white/85 text-stone-900 focus:border-amber-500 focus:ring-4 focus:ring-amber-100'"
            :placeholder="isRegisterMode ? t('login.setPassword') : t('login.enterPassword')"
            type="password"
          />
        </label>

        <label v-if="!isRegisterMode" class="flex items-center gap-3 text-sm" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
          <input
            v-model="loginForm.keepLogin"
            class="h-4 w-4 rounded border-stone-300 text-amber-700 focus:ring-amber-200"
            type="checkbox"
          />
          <span>{{ t('login.keepLogin') }}</span>
        </label>

        <button
          class="inline-flex items-center justify-center rounded-2xl px-5 py-3 text-sm font-medium transition hover:-translate-y-0.5 disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:translate-y-0"
          :class="isDark ? 'bg-stone-100 text-stone-900 hover:bg-white' : 'bg-stone-900 text-white hover:bg-stone-800'"
          :disabled="busyLogin"
          type="submit"
        >
          {{ busyLogin ? (isRegisterMode ? t('login.registering') : t('login.loggingIn')) : (isRegisterMode ? t('login.register') : t('login.login')) }}
        </button>
      </form>

      <p class="mt-4 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        <template v-if="isRegisterMode">
          {{ t('login.hasAccount') }}
          <button class="font-medium text-amber-600 hover:underline" @click="isRegisterMode = false">{{ t('login.backToLogin') }}</button>
        </template>
        <template v-else>
          {{ t('login.noAccount') }}
          <button class="font-medium text-amber-600 hover:underline" @click="isRegisterMode = true">{{ t('login.register') }}</button>
        </template>
      </p>
    </section>
  </div>
</template>
