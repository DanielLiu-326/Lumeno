export const WALLPAPER_PRESETS = [
  {
    id: "lumen",
    swatch:
      "radial-gradient(circle at 18% 12%, rgba(177, 77, 40, 0.42), transparent 42%), linear-gradient(160deg, #f8f3ec 0%, #f4efe7 46%, #efe4d6 100%)",
    light:
      "radial-gradient(circle at top left, rgba(177, 77, 40, 0.18), transparent 28%), radial-gradient(circle at top right, rgba(29, 106, 97, 0.18), transparent 22%), linear-gradient(160deg, #f8f3ec 0%, #f4efe7 44%, #efe4d6 100%)",
    dark:
      "radial-gradient(circle at top left, rgba(251, 146, 60, 0.12), transparent 32%), radial-gradient(circle at top right, rgba(20, 184, 166, 0.12), transparent 26%), linear-gradient(160deg, #111827 0%, #0f172a 44%, #020617 100%)",
  },
  {
    id: "dusk",
    swatch:
      "radial-gradient(circle at 80% 0%, rgba(244, 114, 82, 0.45), transparent 46%), linear-gradient(160deg, #f6e7dc 0%, #edd5c4 100%)",
    light:
      "radial-gradient(circle at top left, rgba(190, 74, 46, 0.22), transparent 30%), radial-gradient(circle at bottom right, rgba(124, 45, 82, 0.16), transparent 34%), linear-gradient(165deg, #f7ece3 0%, #f0d9c8 48%, #e7c4b0 100%)",
    dark:
      "radial-gradient(circle at top left, rgba(251, 113, 133, 0.16), transparent 34%), radial-gradient(circle at bottom right, rgba(249, 115, 22, 0.14), transparent 36%), linear-gradient(165deg, #1c1214 0%, #140e12 46%, #0b0708 100%)",
  },
  {
    id: "mist",
    swatch:
      "radial-gradient(circle at 20% 0%, rgba(100, 140, 170, 0.4), transparent 44%), linear-gradient(160deg, #eef3f6 0%, #d9e3ea 100%)",
    light:
      "radial-gradient(circle at top left, rgba(71, 112, 140, 0.18), transparent 32%), radial-gradient(circle at top right, rgba(148, 163, 184, 0.2), transparent 28%), linear-gradient(160deg, #f3f6f8 0%, #e7eef3 48%, #d7e2ea 100%)",
    dark:
      "radial-gradient(circle at top left, rgba(125, 211, 252, 0.1), transparent 34%), radial-gradient(circle at top right, rgba(148, 163, 184, 0.12), transparent 30%), linear-gradient(160deg, #0f1720 0%, #111827 48%, #020617 100%)",
  },
  {
    id: "meadow",
    swatch:
      "radial-gradient(circle at 78% 12%, rgba(52, 140, 96, 0.38), transparent 44%), linear-gradient(160deg, #eef6ef 0%, #d7ead8 100%)",
    light:
      "radial-gradient(circle at top left, rgba(47, 122, 78, 0.16), transparent 30%), radial-gradient(circle at top right, rgba(180, 140, 60, 0.14), transparent 26%), linear-gradient(160deg, #f3f7f1 0%, #e7f0e4 46%, #d7e6d2 100%)",
    dark:
      "radial-gradient(circle at top left, rgba(52, 211, 153, 0.1), transparent 32%), radial-gradient(circle at top right, rgba(250, 204, 21, 0.08), transparent 28%), linear-gradient(160deg, #0c1612 0%, #0f1a14 46%, #07100c 100%)",
  },
];

const PRESET_IDS = new Set(WALLPAPER_PRESETS.map((item) => item.id));

export function defaultWallpaperSource() {
  return { type: "preset", id: "lumen" };
}

export function defaultWallpaper() {
  return {
    darkMode: "auto",
    source: defaultWallpaperSource(),
    darkSource: defaultWallpaperSource(),
  };
}

function normalizeHex(value) {
  const raw = String(value || "").trim();
  const hex = raw.startsWith("#") ? raw.slice(1) : raw;
  if (hex.length !== 3 && hex.length !== 6) return null;
  if (!/^[0-9a-fA-F]+$/.test(hex)) return null;
  return `#${hex.toLowerCase()}`;
}

export function normalizeWallpaperSource(raw) {
  const src = raw && typeof raw === "object" ? raw : {};
  if (src.type === "image") {
    const file = typeof src.file === "string" ? src.file : "";
    if (file && !file.includes("..") && !file.includes("/") && !file.includes("\\")) {
      return { type: "image", file };
    }
    return { type: "image", file: "" };
  }
  if (src.type === "color") {
    const hex = normalizeHex(src.hex);
    if (hex) return { type: "color", hex };
    return defaultWallpaperSource();
  }
  const id = PRESET_IDS.has(src.id) ? src.id : "lumen";
  return { type: "preset", id };
}

export function normalizeWallpaper(raw) {
  const value = raw && typeof raw === "object" ? raw : {};
  return {
    darkMode: value.darkMode === "separate" ? "separate" : "auto",
    source: normalizeWallpaperSource(value.source),
    darkSource: normalizeWallpaperSource(value.darkSource || value.source),
  };
}

export function resolveWallpaperSource(prefs, isDark) {
  const wallpaper = normalizeWallpaper(prefs);
  if (isDark && wallpaper.darkMode === "separate") {
    return wallpaper.darkSource;
  }
  return wallpaper.source;
}

export function wallpaperImageUrl(file) {
  if (!file) return "";
  return `/api/wallpaper/${encodeURIComponent(file)}`;
}

export function wallpaperBackgroundStyle(source, { variant = "light", dim = false } = {}) {
  const src = normalizeWallpaperSource(source);
  if (src.type === "image" && src.file) {
    return {
      backgroundImage: `url("${wallpaperImageUrl(src.file)}")`,
      backgroundSize: "cover",
      backgroundPosition: "center",
      backgroundRepeat: "no-repeat",
      filter: dim ? "brightness(0.42) saturate(0.78)" : "none",
    };
  }
  if (src.type === "color" && src.hex) {
    if (dim) {
      return {
        background: `linear-gradient(rgba(2, 6, 23, 0.62), rgba(2, 6, 23, 0.62)), ${src.hex}`,
      };
    }
    return { background: src.hex };
  }
  const preset = WALLPAPER_PRESETS.find((item) => item.id === src.id) || WALLPAPER_PRESETS[0];
  return { background: variant === "dark" ? preset.dark : preset.light };
}

export function wallpaperLayerState(prefs, isDark) {
  const wallpaper = normalizeWallpaper(prefs);
  const source = resolveWallpaperSource(wallpaper, isDark);
  const autoDim = Boolean(isDark) && wallpaper.darkMode !== "separate";
  return {
    source,
    showGrid: source.type === "preset" && source.id === "lumen",
    style: wallpaperBackgroundStyle(source, {
      variant: isDark && source.type === "preset" ? "dark" : "light",
      dim: autoDim && source.type !== "preset",
    }),
  };
}

export function wallpaperPreviewStyle(source, isDark) {
  const src = normalizeWallpaperSource(source);
  if (src.type === "image" && src.file) {
    return {
      backgroundImage: `url("${wallpaperImageUrl(src.file)}")`,
      backgroundSize: "cover",
      backgroundPosition: "center",
    };
  }
  if (src.type === "color" && src.hex) {
    return { background: src.hex };
  }
  const preset = WALLPAPER_PRESETS.find((item) => item.id === src.id) || WALLPAPER_PRESETS[0];
  return { background: isDark ? preset.dark : preset.light };
}
