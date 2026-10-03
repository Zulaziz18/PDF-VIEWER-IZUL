import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "@/design/tokens.css";
import { applySystemTheme } from "@/design/theme";

applySystemTheme();

// Ctrl+wheel — and a touchpad pinch, which Chromium reports the same way — is
// never the browser's: with WebView2 pinch zoom on (src-tauri/src/pinch.rs) an
// uncancelled one would scale the whole interface. Over a page the viewport
// turns it into a document zoom; anywhere else it does nothing.
window.addEventListener(
  "wheel",
  (e) => {
    if (e.ctrlKey || e.metaKey) e.preventDefault();
  },
  { passive: false },
);

const host = document.getElementById("root");
if (!host) {
  throw new Error("elemen #root tidak ditemukan");
}
createRoot(host).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
