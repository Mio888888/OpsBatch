import assert from 'node:assert/strict';
import test from 'node:test';
import { drainRdpFrameBatch } from '../src/pages/Rdp/rdpProtocol.ts';

test('drains a bounded RDP frame batch while preserving queue order', () => {
  const queue = [1, 2, 3, 4, 5];

  assert.deepEqual(drainRdpFrameBatch(queue, 2), [1, 2]);
  assert.deepEqual(queue, [3, 4, 5]);
});

test('drains all pending RDP frames when queue is below the paint budget', () => {
  const queue = [1, 2];

  assert.deepEqual(drainRdpFrameBatch(queue, 4), [1, 2]);
  assert.deepEqual(queue, []);
});

test('ignores invalid RDP frame paint budgets', () => {
  const queue = [1, 2];

  assert.deepEqual(drainRdpFrameBatch(queue, 0), []);
  assert.deepEqual(drainRdpFrameBatch(queue, Number.NaN), []);
  assert.deepEqual(queue, [1, 2]);
});
