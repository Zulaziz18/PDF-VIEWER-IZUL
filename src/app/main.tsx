import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "@/design/tokens.css";

const host = document.getElementById("root");
if (!host) {
  throw new Error("elemen #root tidak ditemukan");
}
createRoot(host).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
