import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('Popconfirm confirmation stops propagation before running the callback', () => {
  const source = readFileSync('src/components/ui/index.tsx', 'utf8');

  assert.match(
    source,
    /onClick=\{\(event\) => \{\s*event\.stopPropagation\(\);\s*onConfirm\?\.\(\);\s*\}\}/,
  );
});
