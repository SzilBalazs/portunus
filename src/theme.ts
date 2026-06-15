import { invoke } from "@tauri-apps/api/core";
import type { Config } from "./types";

const MATUGEN_THEME = "matugen";
const MATUGEN_STYLE_ID = "matugen-theme";

/** Fetch the external matugen.css from the backend and inject it as a <style>
 *  element. The CSS is scoped to `:root[data-theme="matugen"]`, so it only takes
 *  effect when that theme is active and is harmless to leave in the document.
 *  Missing/empty file → no rules → vars fall back to App.css :root defaults. */
export async function injectMatugenTheme() {
  const css = (await invoke<string | null>("get_custom_theme_css")) ?? "";
  let el = document.getElementById(MATUGEN_STYLE_ID) as HTMLStyleElement | null;
  if (!el) {
    el = document.createElement("style");
    el.id = MATUGEN_STYLE_ID;
    document.head.appendChild(el);
  }
  el.textContent = css;
}

export function applyTheme(appearance: Config["appearance"]) {
  const root = document.documentElement;
  root.setAttribute("data-theme", appearance.theme);
  root.style.zoom = String(appearance.font_size / 13);
  root.dataset.animateResults = String(appearance.animate_results ?? "slide");
  root.dataset.showMetadata = String(appearance.show_metadata ?? true);
  root.dataset.accentBleed = String(appearance.accent_bleed ?? true);
  root.dataset.slideSelection = String(appearance.slide_selection ?? true);
  root.style.setProperty("--grain-opacity", String(appearance.grain ?? 0.07));
  if (appearance.theme === MATUGEN_THEME) void injectMatugenTheme();
}
