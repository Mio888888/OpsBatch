import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { loadThemeSettings, registerThemeSettingsSync } from "./stores/theme";
import { loadLanguageSettings, registerLanguageSettingsSync } from "./stores/language";
import { emit } from "@tauri-apps/api/event";
import { resolveSystemLanguage, translateText } from "./i18n";

registerThemeSettingsSync();
loadThemeSettings();
registerLanguageSettingsSync();
loadLanguageSettings();

document.addEventListener("contextmenu", (event) => {
  event.preventDefault();
});

// ---------------------------------------------------------------------------
// Frontend log bridge: intercept console and emit as Tauri events so the
// global-log window (separate JS context) can display them.
// ---------------------------------------------------------------------------

function frontendTimestamp(): string {
  const d = new Date();
  const t = d.toTimeString().split(" ")[0];
  return `${t}.${String(d.getMilliseconds()).padStart(3, "0")}`;
}

function emitFrontendLog(level: string, source: string, message: string) {
  emit("global-log", {
    timestamp: frontendTimestamp(),
    level,
    source,
    message,
    origin: "frontend",
  }).catch(() => {});
}

const origLog = console.log;
const origWarn = console.warn;
const origError = console.error;

console.log = (...args: unknown[]) => {
  origLog(...args);
  try { emitFrontendLog("info", "console", args.map(String).join(" ")); } catch {}
};

console.warn = (...args: unknown[]) => {
  origWarn(...args);
  try { emitFrontendLog("warn", "console", args.map(String).join(" ")); } catch {}
};

console.error = (...args: unknown[]) => {
  origError(...args);
  try { emitFrontendLog("error", "console", args.map(String).join(" ")); } catch {}
};

window.addEventListener("error", (event) => {
  emitFrontendLog("error", "window", `${event.message} (${event.filename}:${event.lineno})`);
});

window.addEventListener("unhandledrejection", (event) => {
  emitFrontendLog("error", "promise", String(event.reason));
});

emitFrontendLog("info", "system", translateText(resolveSystemLanguage(), 'app.started'));

// ---------------------------------------------------------------------------

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
