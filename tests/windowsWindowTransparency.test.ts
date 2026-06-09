import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

type TauriWindowConfig = {
  transparent?: boolean;
  backgroundColor?: string;
};

type TauriConfig = {
  app?: {
    windows?: TauriWindowConfig[];
  };
};

test('base main window keeps transparent background for macOS rounded corners', () => {
  const config = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')) as TauriConfig;
  const mainWindow = config.app?.windows?.[0];

  assert.ok(mainWindow, 'main window should be configured');
  assert.equal(mainWindow.transparent, true);
  assert.equal(mainWindow.backgroundColor?.toLowerCase(), '#00000000');
});

test('windows main window override uses an opaque background', () => {
  const config = JSON.parse(
    readFileSync('src-tauri/tauri.windows.conf.json', 'utf8'),
  ) as TauriConfig;
  const mainWindow = config.app?.windows?.[0];

  assert.ok(mainWindow, 'windows main window should override the base config');
  assert.equal(mainWindow.transparent, false);
  assert.equal(mainWindow.backgroundColor?.toLowerCase(), '#e8e3d6');
});

test('managed child windows only enable transparent WebViews on macOS', () => {
  const source = readFileSync('src-tauri/src/commands/window.rs', 'utf8');

  assert.match(source, /#\[cfg\(target_os = "macos"\)\][\s\S]*\.transparent\(true\)/);
  assert.match(source, /#\[cfg\(target_os = "macos"\)\][\s\S]*\.background_color\(\s*tauri::utils::config::Color\(0,\s*0,\s*0,\s*0\)\s*\)/);
});

test('managed child windows load the bundled SPA entry before routing', () => {
  const windowSource = readFileSync('src-tauri/src/commands/window.rs', 'utf8');
  const appSource = readFileSync('src/App.tsx', 'utf8');

  assert.match(windowSource, /fn spa_entry_for_route\(route: &str\) -> String\s*\{\s*format!\("index\.html\?route=\{\}", encode_route\(route\)\)\s*\}/s);
  assert.match(windowSource, /let url = spa_entry_for_route\(&route\);[\s\S]*WebviewUrl::App\(url\.into\(\)\)/);
  assert.match(windowSource, /index\.html\?route=/);
  assert.doesNotMatch(windowSource, /index\.html#/);
  assert.doesNotMatch(windowSource, /WebviewUrl::App\(route\.into\(\)\)/);
  assert.match(appSource, /import \{[^}]*HashRouter[^}]*\} from 'react-router-dom'/);
  assert.match(appSource, /applyInitialRouteToHash\(\);/);
  assert.match(appSource, /window\.history\.replaceState\(null, '', window\.location\.pathname\)/);
  assert.match(appSource, /window\.location\.hash = route/);
  assert.doesNotMatch(appSource, /consumeInitialRouteFromSearch/);
  assert.match(appSource, /<HashRouter>/);
});

test('managed child windows are created from an async command on Windows', () => {
  const windowSource = readFileSync('src-tauri/src/commands/window.rs', 'utf8');

  assert.match(windowSource, /#\[tauri::command\]\s*pub async fn open_managed_window/);
  assert.match(windowSource, /tauri::async_runtime::spawn_blocking/);
});

test('windows uses square app chrome while macOS keeps the rounded default', () => {
  const mainSource = readFileSync('src/main.tsx', 'utf8');
  const cssSource = readFileSync('src/App.css', 'utf8');

  assert.match(mainSource, /document\.documentElement\.dataset\.platform = detectHostPlatform\(\)/);
  assert.match(cssSource, /--window-radius:\s*20px/);
  assert.match(cssSource, /\[data-platform="windows"\][\s\S]*--window-radius:\s*0/);
});
