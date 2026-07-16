import { reactive, computed } from "vue";
import en from "./en.js";
import zh from "./zh.js";

const messages = { en, zh };

const state = reactive({
    locale: "en",
});

export function t(key, params) {
    const lang = messages[state.locale] || messages.en;
    let text = lang[key] ?? messages.en[key] ?? key;
    if (params) {
        for (const [k, v] of Object.entries(params)) {
            text = text.replaceAll(`{${k}}`, v);
        }
    }
    return text;
}

export function setLocale(loc) {
    state.locale = loc;
    document.documentElement.lang = loc === "zh" ? "zh-CN" : "en";
}

setLocale(state.locale);

export const locale = computed(() => state.locale);
export const availableLocales = ["en", "zh"];
