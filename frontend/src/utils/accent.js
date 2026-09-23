const STEPS = [50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950];
const ORANGE_SHIFT = {
  50: 100,
  100: 200,
  200: 300,
  300: 400,
  400: 500,
  500: 600,
  600: 700,
  700: 800,
  800: 900,
  900: 950,
  950: 950,
};

export const ACCENT_PRESETS = [
  {
    id: "amber",
    scale: {
      50: "#fffbeb",
      100: "#fef3c7",
      200: "#fde68a",
      300: "#fcd34d",
      400: "#fbbf24",
      500: "#f59e0b",
      600: "#d97706",
      700: "#b45309",
      800: "#92400e",
      900: "#78350f",
      950: "#451a03",
    },
  },
  {
    id: "orange",
    scale: {
      50: "#fff7ed",
      100: "#ffedd5",
      200: "#fed7aa",
      300: "#fdba74",
      400: "#fb923c",
      500: "#f97316",
      600: "#ea580c",
      700: "#c2410c",
      800: "#9a3412",
      900: "#7c2d12",
      950: "#431407",
    },
  },
  {
    id: "rose",
    scale: {
      50: "#fff1f2",
      100: "#ffe4e6",
      200: "#fecdd3",
      300: "#fda4af",
      400: "#fb7185",
      500: "#f43f5e",
      600: "#e11d48",
      700: "#be123c",
      800: "#9f1239",
      900: "#881337",
      950: "#4c0519",
    },
  },
  {
    id: "violet",
    scale: {
      50: "#f5f3ff",
      100: "#ede9fe",
      200: "#ddd6fe",
      300: "#c4b5fd",
      400: "#a78bfa",
      500: "#8b5cf6",
      600: "#7c3aed",
      700: "#6d28d9",
      800: "#5b21b6",
      900: "#4c1d95",
      950: "#2e1065",
    },
  },
  {
    id: "sky",
    scale: {
      50: "#f0f9ff",
      100: "#e0f2fe",
      200: "#bae6fd",
      300: "#7dd3fc",
      400: "#38bdf8",
      500: "#0ea5e9",
      600: "#0284c7",
      700: "#0369a1",
      800: "#075985",
      900: "#0c4a6e",
      950: "#082f49",
    },
  },
  {
    id: "teal",
    scale: {
      50: "#f0fdfa",
      100: "#ccfbf1",
      200: "#99f6e4",
      300: "#5eead4",
      400: "#2dd4bf",
      500: "#14b8a6",
      600: "#0d9488",
      700: "#0f766e",
      800: "#115e59",
      900: "#134e4a",
      950: "#042f2e",
    },
  },
  {
    id: "emerald",
    scale: {
      50: "#ecfdf5",
      100: "#d1fae5",
      200: "#a7f3d0",
      300: "#6ee7b7",
      400: "#34d399",
      500: "#10b981",
      600: "#059669",
      700: "#047857",
      800: "#065f46",
      900: "#064e3b",
      950: "#022c22",
    },
  },
  {
    id: "blue",
    scale: {
      50: "#eff6ff",
      100: "#dbeafe",
      200: "#bfdbfe",
      300: "#93c5fd",
      400: "#60a5fa",
      500: "#3b82f6",
      600: "#2563eb",
      700: "#1d4ed8",
      800: "#1e40af",
      900: "#1e3a8a",
      950: "#172554",
    },
  },
];

const PRESET_IDS = new Set(ACCENT_PRESETS.map((item) => item.id));
const PRESET_BY_SWATCH = new Map(
  ACCENT_PRESETS.map((item) => [item.scale[500], item.id]),
);

export function defaultAccent() {
  return "amber";
}

function normalizeHex(value) {
  const raw = String(value || "").trim();
  const hex = raw.startsWith("#") ? raw.slice(1) : raw;
  if (hex.length !== 3 && hex.length !== 6) return null;
  if (!/^[0-9a-fA-F]+$/.test(hex)) return null;
  const full = hex.length === 3 ? hex.split("").map((ch) => ch + ch).join("") : hex;
  return `#${full.toLowerCase()}`;
}

export function normalizeAccent(raw) {
  const value = String(raw || "").trim().toLowerCase();
  if (PRESET_IDS.has(value)) return value;
  return normalizeHex(value) || defaultAccent();
}

function hexToRgb(hex) {
  const value = normalizeHex(hex) || "#f59e0b";
  return {
    r: parseInt(value.slice(1, 3), 16),
    g: parseInt(value.slice(3, 5), 16),
    b: parseInt(value.slice(5, 7), 16),
  };
}

function rgbToHex({ r, g, b }) {
  return `#${[r, g, b].map((n) => n.toString(16).padStart(2, "0")).join("")}`;
}

function mix(from, to, amount) {
  return {
    r: Math.round(from.r + (to.r - from.r) * amount),
    g: Math.round(from.g + (to.g - from.g) * amount),
    b: Math.round(from.b + (to.b - from.b) * amount),
  };
}

function scaleFromHex(hex) {
  const base = hexToRgb(hex);
  const white = { r: 255, g: 255, b: 255 };
  const black = { r: 0, g: 0, b: 0 };
  return {
    50: rgbToHex(mix(base, white, 0.92)),
    100: rgbToHex(mix(base, white, 0.84)),
    200: rgbToHex(mix(base, white, 0.68)),
    300: rgbToHex(mix(base, white, 0.48)),
    400: rgbToHex(mix(base, white, 0.24)),
    500: normalizeHex(hex),
    600: rgbToHex(mix(base, black, 0.16)),
    700: rgbToHex(mix(base, black, 0.32)),
    800: rgbToHex(mix(base, black, 0.48)),
    900: rgbToHex(mix(base, black, 0.62)),
    950: rgbToHex(mix(base, black, 0.78)),
  };
}

export function accentScale(value) {
  const accent = normalizeAccent(value);
  const preset = ACCENT_PRESETS.find((item) => item.id === accent);
  if (preset) return preset.scale;
  return scaleFromHex(accent);
}

export function accentSwatch(value) {
  return accentScale(value)[500];
}

export function isPresetAccent(value) {
  return PRESET_IDS.has(normalizeAccent(value));
}

export function snapAccent(value) {
  const accent = normalizeAccent(value);
  if (PRESET_IDS.has(accent)) return accent;
  return PRESET_BY_SWATCH.get(accent) || accent;
}

export function applyAccent(value) {
  const accent = normalizeAccent(value);
  const scale = accentScale(accent);
  const root = document.documentElement;
  for (const step of STEPS) {
    root.style.setProperty(`--color-amber-${step}`, scale[step]);
    root.style.setProperty(`--color-orange-${step}`, scale[ORANGE_SHIFT[step]]);
  }
  const rgb = hexToRgb(scale[500]);
  root.style.setProperty("--accent", scale[500]);
  root.style.setProperty("--accent-rgb", `${rgb.r} ${rgb.g} ${rgb.b}`);
  try {
    localStorage.setItem("album-accent", accent);
  } catch {
    /* ignore quota / private mode */
  }
  return accent;
}

export function loadCachedAccent() {
  try {
    return normalizeAccent(localStorage.getItem("album-accent"));
  } catch {
    return defaultAccent();
  }
}
