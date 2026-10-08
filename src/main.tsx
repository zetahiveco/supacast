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

// Ask for microphone access as soon as the app launches so dictation never
// fails mid-recording. wry grants the WKWebView-level capture permission by
// default, so this triggers the macOS TCC prompt (backed by
// NSMicrophoneUsageDescription in Info.plist); the grant persists app-wide.
// Only runs inside the Tauri runtime — a plain browser tab would prompt too.
if (label === "main" && "__TAURI_INTERNALS__" in window) {
  navigator.mediaDevices
    ?.getUserMedia({ audio: true })
    .then((stream) => stream.getTracks().forEach((t) => t.stop()))
    .catch(() => {
      // Denied or unsupported — dictation surfaces the error when used.
    });
}

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
