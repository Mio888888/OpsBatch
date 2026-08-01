import assert from 'node:assert/strict';
import test from 'node:test';
import { lightThemes } from '../src/data/themes.ts';
import { UI_TEXT_PALETTES } from '../src/utils/uiReadability.ts';
import {
  defaultUiScaleForPlatform,
  normalizeUiScale,
  WINDOWS_DEFAULT_UI_SCALE,
} from '../src/utils/uiScale.ts';

function relativeLuminance(hex: string) {
  const normalized = hex.length === 4
    ? `#${hex.slice(1).split('').map((character) => character.repeat(2)).join('')}`
    : hex;
  const channels = [1, 3, 5].map((offset) => Number.parseInt(normalized.slice(offset, offset + 2), 16) / 255);
  const linear = channels.map((channel) => (
    channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4
  ));
  return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
}

function contrastRatio(foreground: string, background: string) {
  const foregroundLuminance = relativeLuminance(foreground);
  const backgroundLuminance = relativeLuminance(background);
  return (Math.max(foregroundLuminance, backgroundLuminance) + 0.05)
    / (Math.min(foregroundLuminance, backgroundLuminance) + 0.05);
}

test('Windows defaults to a readable 115 percent interface scale', () => {
  assert.equal(defaultUiScaleForPlatform('Windows NT 10.0'), WINDOWS_DEFAULT_UI_SCALE);
  assert.equal(defaultUiScaleForPlatform('MacIntel'), 100);
  assert.equal(normalizeUiScale('117'), 115);
  assert.equal(normalizeUiScale(999), 150);
});

test('light theme secondary and muted UI text stay above 4.5:1 contrast', () => {
  for (const theme of lightThemes) {
    for (const background of [theme.ui.pageBg, theme.ui.sidebarBg, theme.ui.mainBg, theme.ui.cardBg]) {
      assert.ok(
        contrastRatio(UI_TEXT_PALETTES.light.secondary, background) >= 4.5,
        `${theme.id} secondary text is too faint on ${background}`,
      );
      assert.ok(
        contrastRatio(UI_TEXT_PALETTES.light.muted, background) >= 4.5,
        `${theme.id} muted text is too faint on ${background}`,
      );
    }
  }
});

test('Windows UI uses native fonts and avoids synthetic fractional weights', async () => {
  const { readFile } = await import('node:fs/promises');
  const tokens = await readFile('src/styles/foundation/tokens.css', 'utf8');
  const platform = await readFile('src/styles/foundation/platform.css', 'utf8');

  assert.match(tokens, /--font-sans-windows:\s*'Segoe UI Variable Text',\s*'Segoe UI'/);
  assert.doesNotMatch(tokens, /--font-sans-windows:[^;]*HarmonyOS Sans SC/);
  assert.match(platform, /font-synthesis:\s*none/);
  assert.match(platform, /--font-size-body:\s*14px/);
  assert.match(platform, /--font-weight-strong:\s*600/);
  assert.match(platform, /--font-weight-display:\s*700/);
});

test('Windows application manifest stays ASCII-safe when embedded by the resource compiler', async () => {
  const { readFile } = await import('node:fs/promises');
  const manifest = await readFile('src-tauri/opsbatch.exe.manifest', 'utf8');

  assert.doesNotMatch(manifest, /[^\x00-\x7F]/);
  assert.match(manifest, /<dpiAware[^>]*>true\/pm<\/dpiAware>/);
  assert.match(manifest, /<dpiAwareness[^>]*>PerMonitorV2,PerMonitor<\/dpiAwareness>/);
});
