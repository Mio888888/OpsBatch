export interface LocalPerformanceSnapshot {
  cpuTimeMs?: number;
  memoryRssMb?: number;
  logicalCpuCount?: number;
  timestamp?: number;
}

const PROCESS_MEMORY_BAR_MAX_MB = 1024;

export function clampMetricPercent(value?: number) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}

export function calculateProcessCpuPercent(
  previous: LocalPerformanceSnapshot | null | undefined,
  current: LocalPerformanceSnapshot | null | undefined,
) {
  if (
    typeof previous?.cpuTimeMs !== 'number'
    || typeof current?.cpuTimeMs !== 'number'
    || typeof previous.timestamp !== 'number'
    || typeof current.timestamp !== 'number'
    || typeof current.logicalCpuCount !== 'number'
    || current.logicalCpuCount <= 0
  ) return undefined;

  const elapsedMs = current.timestamp - previous.timestamp;
  const cpuDeltaMs = current.cpuTimeMs - previous.cpuTimeMs;
  if (elapsedMs <= 0 || cpuDeltaMs < 0) return undefined;

  return clampMetricPercent((cpuDeltaMs / elapsedMs / current.logicalCpuCount) * 100);
}

export function formatProcessMemory(memoryRssMb?: number) {
  if (typeof memoryRssMb !== 'number' || !Number.isFinite(memoryRssMb) || memoryRssMb < 0) return '--';
  if (memoryRssMb >= 1024) return `${(memoryRssMb / 1024).toFixed(1)}G`;
  return `${Math.round(memoryRssMb)}M`;
}

export function getProcessMemoryBarPercent(memoryRssMb?: number) {
  if (typeof memoryRssMb !== 'number' || !Number.isFinite(memoryRssMb) || memoryRssMb < 0) return 0;
  return clampMetricPercent((memoryRssMb / PROCESS_MEMORY_BAR_MAX_MB) * 100);
}
