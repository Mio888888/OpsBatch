import assert from 'node:assert/strict';
import test from 'node:test';
import {
  clampMetricPercent,
  calculateProcessCpuPercent,
  formatProcessMemory,
  getProcessMemoryBarPercent,
} from '../src/utils/localPerformance.ts';

test('clamps metric percentage values to the visible bar range', () => {
  assert.equal(clampMetricPercent(-8), 0);
  assert.equal(clampMetricPercent(48.4), 48.4);
  assert.equal(clampMetricPercent(180), 100);
});

test('returns zero for missing or invalid metric percentages', () => {
  assert.equal(clampMetricPercent(undefined), 0);
  assert.equal(clampMetricPercent(Number.NaN), 0);
});

test('calculates process cpu percentage from cpu-time deltas', () => {
  assert.equal(
    calculateProcessCpuPercent(
      { cpuTimeMs: 1200, timestamp: 1000, logicalCpuCount: 4 },
      { cpuTimeMs: 2200, timestamp: 2000, logicalCpuCount: 4 },
    ),
    25,
  );
});

test('returns undefined process cpu percentage for incomplete snapshots', () => {
  assert.equal(
    calculateProcessCpuPercent(
      { cpuTimeMs: 1200, timestamp: 1000, logicalCpuCount: 4 },
      { timestamp: 2000, logicalCpuCount: 4 },
    ),
    undefined,
  );
});

test('formats process memory as a compact app usage value', () => {
  assert.equal(formatProcessMemory(384), '384M');
  assert.equal(formatProcessMemory(1536), '1.5G');
});

test('formats unknown process memory as placeholder', () => {
  assert.equal(formatProcessMemory(undefined), '--');
});

test('maps process memory to a bounded visual percentage', () => {
  assert.equal(getProcessMemoryBarPercent(128), 12.5);
  assert.equal(getProcessMemoryBarPercent(2048), 100);
});
