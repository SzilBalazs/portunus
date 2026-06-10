import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import Settings from "./Settings";

const root = document.getElementById("root") as HTMLElement;
const isSettings = getCurrentWindow().label === "settings";

// Mark the document so window-specific CSS (e.g. the opaque settings surface)
// doesn't leak into the transparent launcher window, which shares this bundle.
if (isSettings) document.documentElement.classList.add("settings-win");

// Kill the native WebKit context menu (open link / open in new window / inspect).
// Opening a link there navigates the webview away and destroys the app.
window.addEventListener("contextmenu", e => e.preventDefault());

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    {isSettings ? <Settings /> : <App />}
  </React.StrictMode>,
);
