import { useEffect, useState } from "react";
import {
  ACCENT_CHANGE_EVENT,
  ACCENT_STORAGE_KEY,
  applyAccentPreference,
  loadAccentPreference,
  normalizeAccent,
  persistAccentPreference,
} from "../../../../frontend/src/ui/accent";

export const APPEARANCE_STORAGE_KEY = "webcodex.desktop.appearance.v1";
const APPEARANCE_CHANGE_EVENT = "webcodex:appearance-change";
export const APPEARANCES = ["system", "light", "dark"] as const;
export type Appearance = typeof APPEARANCES[number];

function storedAppearance(): Appearance {
  try {
    const value = window.localStorage.getItem(APPEARANCE_STORAGE_KEY);
    return APPEARANCES.includes(value as Appearance) ? value as Appearance : "system";
  } catch {
    return "system";
  }
}

export function useAppearance() {
  const [appearance, setAppearanceState] = useState<Appearance>(storedAppearance);
  const [accent, setAccentState] = useState(loadAccentPreference);

  useEffect(() => {
    const sync = (event: Event) => {
      const next = (event as CustomEvent<Appearance>).detail;
      if (APPEARANCES.includes(next)) setAppearanceState(next);
    };
    window.addEventListener(APPEARANCE_CHANGE_EVENT, sync);
    return () => window.removeEventListener(APPEARANCE_CHANGE_EVENT, sync);
  }, []);

  useEffect(() => {
    const sync = (event: Event) => {
      const next = event instanceof StorageEvent ? event.key === ACCENT_STORAGE_KEY ? normalizeAccent(event.newValue) : null : normalizeAccent((event as CustomEvent<string>).detail);
      if (next) setAccentState(next);
    };
    window.addEventListener(ACCENT_CHANGE_EVENT, sync);
    window.addEventListener("storage", sync);
    return () => { window.removeEventListener(ACCENT_CHANGE_EVENT, sync); window.removeEventListener("storage", sync); };
  }, []);

  useEffect(() => {
    const media = typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: dark)")
      : null;
    const apply = () => {
      const theme = appearance === "system" ? (media?.matches ? "dark" : "light") : appearance;
      document.documentElement.dataset.theme = theme;
      document.documentElement.dataset.appearance = appearance;
      document.documentElement.dataset.mantineColorScheme = theme;
      applyAccentPreference(accent, theme);
      document.querySelector('meta[name="theme-color"]')?.setAttribute(
        "content",
        theme === "dark" ? "#0b0b0c" : "#f5f5f5",
      );
    };
    apply();
    media?.addEventListener("change", apply);
    try {
      window.localStorage.setItem(APPEARANCE_STORAGE_KEY, appearance);
    } catch {
      // Appearance is a local convenience and remains usable without storage.
    }
    return () => media?.removeEventListener("change", apply);
  }, [appearance, accent]);

  const setAppearance = (next: Appearance) => {
    setAppearanceState(next);
    window.dispatchEvent(new CustomEvent<Appearance>(APPEARANCE_CHANGE_EVENT, { detail: next }));
  };

  const setAccent = (next: string) => {
    const color = normalizeAccent(next);
    if (!color) return;
    setAccentState(color);
    persistAccentPreference(color);
  };

  return { appearance, setAppearance, accent, setAccent };
}
