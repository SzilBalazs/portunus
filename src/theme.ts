import type { Config } from "./types";

export function applyTheme(appearance: Config["appearance"]) {
  const root = document.documentElement;
  root.setAttribute("data-theme", appearance.theme);
  root.style.zoom = String(appearance.font_size / 13);
  root.dataset.animateResults = String(appearance.animate_results ?? true);
  root.dataset.showMetadata = String(appearance.show_metadata ?? true);
}
