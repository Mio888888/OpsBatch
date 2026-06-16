import assert from 'node:assert/strict';
import test from 'node:test';
import {
  isHostMonitorIdle,
  shouldPollHostMonitor,
} from '../src/utils/hostMonitor.ts';

test('keeps host monitor polling disabled by default', () => {
  assert.equal(shouldPollHostMonitor({ realtimeEnabled: false, hasSnapshot: false }), false);
  assert.equal(isHostMonitorIdle({ realtimeEnabled: false, hasSnapshot: false }), true);
});

test('starts host monitor polling after realtime refresh is enabled', () => {
  assert.equal(shouldPollHostMonitor({ realtimeEnabled: true, hasSnapshot: false }), true);
  assert.equal(isHostMonitorIdle({ realtimeEnabled: true, hasSnapshot: false }), false);
});
