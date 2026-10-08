import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface Settings {
  shortcut: string;
  openai_api_key: string;
  chat_base_url: string;
  chat_model: string;
}

const isMac = navigator.platform.toLowerCase().includes("mac");

/** Pretty-print a shortcut string for display. */
function pretty(shortcut: string): string {
  return shortcut
    .split("+")
    .map((p) => {
      const l = p.toLowerCase();
      if (l === "cmdorctrl") return isMac ? "⌘" : "Ctrl";
      if (l === "cmd" || l === "command") return "⌘";
      if (l === "ctrl" || l === "control") return "Ctrl";
      if (l === "shift") return "⇧";
      if (l === "alt" || l === "option" || l === "opt") return isMac ? "⌥" : "Alt";
      return p.toUpperCase();
    })
    .join(" + ");
}

/** Build a "Cmd/Ctrl+Shift+Y" style string from a keyboard event. */
function comboFromEvent(e: KeyboardEvent): string | null {
  const key = e.key;
  if (["Control", "Meta", "Alt", "Shift", "CapsLock"].includes(key)) {
    return null; // modifiers alone don't complete a combo
  }
  const mods: string[] = [];
  if (isMac) {
    if (e.metaKey) mods.push("Cmd");
    if (e.ctrlKey) mods.push("Ctrl");
  } else {
    if (e.ctrlKey) mods.push("Ctrl");
    if (e.metaKey) mods.push("Cmd");
  }
  if (e.shiftKey) mods.push("Shift");
  if (e.altKey) mods.push("Alt");

  const named = key === " " ? "Space" : key.length === 1 ? key.toUpperCase() : key;
  return [...mods, named].join("+");
}

export default function Settings() {
  const [shortcut, setShortcut] = useState<string>("");
  const [apiKey, setApiKey] = useState<string>("");
  const [recording, setRecording] = useState(false);
  const [status, setStatus] = useState<"idle" | "saved" | "error">("idle");
  const [error, setError] = useState("");
  const statusTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    invoke<Settings>("get_settings")
      .then((s) => {
        setShortcut(s.shortcut);
        setApiKey(s.openai_api_key || "");
      })
      .catch((e) => console.error(e));
    const unlisten = listen("settings-shown", () => {
      setStatus("idle");
    });
    return () => {
      unlisten.then((fn) => fn());
      window.clearTimeout(statusTimer.current);
    };
  }, []);

  // Global key capture while recording a new shortcut.
  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecording(false);
        return;
      }
      const combo = comboFromEvent(e);
      if (combo) {
        setShortcut(combo);
        setRecording(false);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording]);

  const save = async () => {
    try {
      // Endpoint/model are handled by backend defaults; pass nothing.
      const updated = await invoke<Settings>("save_settings", {
        shortcut,
        openaiApiKey: apiKey,
        chatBaseUrl: null,
        chatModel: null,
      });
      setShortcut(updated.shortcut);
      setStatus("saved");
    } catch (e) {
      setStatus("error");
      setError(String(e));
    }
    window.clearTimeout(statusTimer.current);
    statusTimer.current = window.setTimeout(() => setStatus("idle"), 2500);
  };

  const win = getCurrentWindow();

  return (
    <div className="settings">
      <header className="settings-header" data-tauri-drag-region>
        <span className="logo">🚀</span>
        <h1 data-tauri-drag-region>Supacast Settings</h1>
        <button
          className="titlebar-close"
          onClick={() => win.hide()}
          aria-label="Close settings"
        >
          ✕
        </button>
      </header>

      <section className="settings-body">
        <label className="field">
          <span className="field-label">Global shortcut</span>
          <span className="field-hint">
            Press this anywhere to toggle the launcher.
          </span>
          <div className="shortcut-row">
            <button
              className={`shortcut-btn ${recording ? "recording" : ""}`}
              onClick={() => setRecording(true)}
            >
              {recording ? "Press keys… (Esc to cancel)" : pretty(shortcut) || "Set shortcut"}
            </button>
            <button className="save-btn" onClick={save} disabled={recording}>
              Save
            </button>
          </div>
          {status === "saved" && <p className="status ok">Settings updated ✓</p>}
          {status === "error" && <p className="status err">{error}</p>}
        </label>

        <label className="field">
          <span className="field-label">AI — API key</span>
          <span className="field-hint">
            For the chat model, dictation and reminders. Default:
            gpt-5-mini via OpenAI.
          </span>
          <input
            className="api-key-input"
            type="password"
            placeholder="sk-…"
            value={apiKey}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => setApiKey(e.target.value)}
          />
        </label>

        <label className="field">
          <span className="field-label">Defaults</span>
          <span className="field-hint">
            Shortcut: macOS ⌘ + ⇧ + Y, Windows Ctrl + ⇧ + Y.
            Try: “todo”, “remind me to email Adnan at 5pm”, “show me events tomorrow”.
          </span>
        </label>
      </section>

      <footer className="settings-footer">
        <button className="save-btn" onClick={save}>
          Save
        </button>
        <button className="close-btn" onClick={() => win.hide()}>
          Close
        </button>
      </footer>
    </div>
  );
}
