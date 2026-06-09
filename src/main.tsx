import React from "react";
import ReactDOM from "react-dom/client";
import { loadThemeSettings, registerThemeSettingsSync } from "./stores/theme";
import { loadLanguageSettings, registerLanguageSettingsSync } from "./stores/language";
import { emit } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { resolveSystemLanguage, translateText } from "./i18n";

function diagnosticContext() {
  return `href=${window.location.href} search=${window.location.search} hash=${window.location.hash}`;
}

function writeDiagnosticLog(source: string, message: string) {
  const body = `${message} ${diagnosticContext()}`.slice(0, 4000);
  invoke("write_diagnostic_log", { source, message: body }).catch(() => {});
}

function installDiagnosticsBridge() {
  writeDiagnosticLog("frontend", "main entry started");

  window.addEventListener("error", (event) => {
    writeDiagnosticLog("frontend-error", `${event.message} (${event.filename}:${event.lineno})`);
  });

  window.addEventListener("unhandledrejection", (event) => {
    writeDiagnosticLog("frontend-promise", String(event.reason));
  });
}

installDiagnosticsBridge();

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
  const redacted = message
    .replace(/(Bearer\s+)[^\s"']+/gi, '$1***')
    .replace(/(api[_-]?key\s*[=:]\s*)[^\s,"']+/gi, '$1***')
    .replace(/(token\s*[=:]\s*)[^\s,"']+/gi, '$1***')
    .replace(/(password\s*[=:]\s*)[^\s,"']+/gi, '$1***')
    .slice(0, 4000);
  emit("global-log", {
    timestamp: frontendTimestamp(),
    level,
    source,
    message: redacted,
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

void import("./App")
  .then(({ default: App }) => {
    writeDiagnosticLog("frontend", "App module loaded");
    const root = document.getElementById("root");
    if (!root) throw new Error("root element not found");
    ReactDOM.createRoot(root).render(
      <React.StrictMode>
        <App />
      </React.StrictMode>,
    );
    writeDiagnosticLog("frontend", "React render requested");
  })
  .catch((error) => {
    writeDiagnosticLog("frontend-error", `App bootstrap failed: ${String(error)}`);
    throw error;
  });
