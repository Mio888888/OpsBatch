import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('batch terminal and transfer only pass selected Linux hosts', () => {
  const source = readFileSync('src/components/MainLayout.tsx', 'utf8');

  assert.match(source, /function getLinuxHostIds/);
  assert.match(source, /host\.os === 'linux'/);
  assert.match(source, /hostIds: selectedLinuxHostIds/);
  assert.match(source, /disabled=\{selectedHostIds\.length === 0\}/);
  assert.doesNotMatch(source, /kind: 'batch-terminal', hostIds: selectedHostIds/);
  assert.doesNotMatch(source, /kind: 'batch-transfer', hostIds: selectedHostIds/);
});
