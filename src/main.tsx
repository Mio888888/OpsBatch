import React from "react";
import ReactDOM from "react-dom/client";
import { loadThemeSettings, registerThemeSettingsSync } from "./stores/theme";
import { loadLanguageSettings, registerLanguageSettingsSync } from "./stores/language";
import { invoke } from "@tauri-apps/api/core";
import { installGlobalLogHandler, emitFrontendGlobalLog } from "./utils/globalLogger";
import { resolveSystemLanguage, translateText } from "./i18n";

function detectHostPlatform(): string {
  const platformText = `${navigator.userAgent} ${navigator.platform}`.toLowerCase();
  if (platformText.includes("win")) return "windows";
  if (platformText.includes("mac")) return "macos";
  if (platformText.includes("linux")) return "linux";
  return "unknown";
}

document.documentElement.dataset.platform = detectHostPlatform();

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

installGlobalLogHandler();
void emitFrontendGlobalLog("info", "system", translateText(resolveSystemLanguage(), 'app.started'));

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
