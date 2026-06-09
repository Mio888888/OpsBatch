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

test('main window keeps transparent background for rounded window corners', () => {
  const config = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')) as TauriConfig;
  const mainWindow = config.app?.windows?.[0];

  assert.ok(mainWindow, 'main window should be configured');
  assert.equal(mainWindow.transparent, true);
  assert.equal(mainWindow.backgroundColor?.toLowerCase(), '#00000000');
});

test('managed child windows keep transparent WebViews for rounded window corners', () => {
  const source = readFileSync('src-tauri/src/commands/window.rs', 'utf8');

  assert.match(source, /\.transparent\(true\)/);
  assert.match(source, /\.background_color\(\s*tauri::utils::config::Color\(0,\s*0,\s*0,\s*0\)\s*\)/);
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
