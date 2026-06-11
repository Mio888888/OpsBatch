import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { buildRdpSettings } from '../src/utils/rdpSettings.ts';

test('enables remote audio by default for Windows RDP settings', () => {
  const settings = buildRdpSettings({
    rdpDesktopWidth: 1280,
    rdpDesktopHeight: 720,
    rdpEnableClipboard: true,
    rdpMapDisk: false,
  }, 'windows');

  assert.equal(settings?.enableAudio, true);
});

test('keeps an explicit remote audio opt out', () => {
  const settings = buildRdpSettings({
    rdpEnableAudio: false,
  }, 'windows');

  assert.equal(settings?.enableAudio, false);
});

test('uses enabled remote audio defaults in the host form', () => {
  const source = readFileSync('src/components/MainLayout.tsx', 'utf8');

  assert.match(source, /rdpEnableAudio:\s*true/);
  assert.match(source, /setFieldValue\('rdpEnableAudio',\s*true\)/);
  assert.doesNotMatch(source, /rdpEnableAudio:\s*host\.rdpSettings\?\.enableAudio\s*\?\?\s*false/);
});
