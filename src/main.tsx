import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import Launcher from "./launcher";
import Settings from "./settings";
import Dictate from "./dictate";
import "./app.css";

/** Window label, falling back to "main" when loaded outside the Tauri runtime. */
function currentLabel(): string {
  try {
    return getCurrentWindow().label;
  } catch {
    return "main";
  }
}

const label = currentLabel();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {label === "settings" ? (
      <Settings />
    ) : label === "dictate" ? (
      <Dictate />
    ) : (
      <Launcher />
    )}
  </React.StrictMode>,
);
