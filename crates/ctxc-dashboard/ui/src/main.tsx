import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { applyStoredTheme } from "./lib/theme";
import "./index.css";

// Before the first render, so the page never flashes the wrong colours on its
// way to the ones the user chose.
applyStoredTheme();

const root = document.getElementById("root");
if (!root) {
  throw new Error("index.html must provide a #root element");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
