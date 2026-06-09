import assert from 'node:assert/strict';
import test from 'node:test';
import { getTerminalTabCloseTargets } from '../src/utils/terminalTabs.ts';

const tabKeys = ['local', 'web-01', 'db-01', 'cache-01'];

test('selects all terminal tabs for close all', () => {
  assert.deepEqual(getTerminalTabCloseTargets(tabKeys, 'web-01', 'all'), tabKeys);
});

test('selects all tabs except the target for close others', () => {
  assert.deepEqual(getTerminalTabCloseTargets(tabKeys, 'web-01', 'others'), ['local', 'db-01', 'cache-01']);
});

test('selects tabs on the left side of the target', () => {
  assert.deepEqual(getTerminalTabCloseTargets(tabKeys, 'db-01', 'left'), ['local', 'web-01']);
});

test('selects tabs on the right side of the target', () => {
  assert.deepEqual(getTerminalTabCloseTargets(tabKeys, 'db-01', 'right'), ['cache-01']);
});

test('returns no targets when the target tab is missing', () => {
  assert.deepEqual(getTerminalTabCloseTargets(tabKeys, 'missing', 'left'), []);
});
