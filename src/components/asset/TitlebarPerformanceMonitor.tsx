import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../../i18n';
import {
  calculateProcessCpuPercent,
  formatProcessMemory,
  getProcessMemoryBarPercent,
  type LocalPerformanceSnapshot,
} from '../../utils/localPerformance';

export default function TitlebarPerformanceMonitor() {
  const { tText } = useTranslation();
  const [snapshot, setSnapshot] = useState<LocalPerformanceSnapshot | null>(null);
  const [cpuPercent, setCpuPercent] = useState<number | undefined>(undefined);
  const [error, setError] = useState(false);
  const previousSnapshotRef = useRef<LocalPerformanceSnapshot | null>(null);

  useEffect(() => {
    let disposed = false;
    let inFlight = false;

    const loadSnapshot = async () => {
      if (inFlight) return;
      inFlight = true;
      try {
        const next = await invoke<{
          cpu_time_ms?: number | null;
          memory_rss_mb?: number | null;
          logical_cpu_count?: number | null;
          timestamp?: number;
        }>('get_local_performance_snapshot');
        if (disposed) return;
        const normalizedSnapshot: LocalPerformanceSnapshot = {
          cpuTimeMs: next.cpu_time_ms ?? undefined,
          memoryRssMb: next.memory_rss_mb ?? undefined,
          logicalCpuCount: next.logical_cpu_count ?? undefined,
          timestamp: next.timestamp,
        };
        const nextCpuPercent = calculateProcessCpuPercent(previousSnapshotRef.current, normalizedSnapshot);
        previousSnapshotRef.current = normalizedSnapshot;
        setSnapshot(normalizedSnapshot);
        setCpuPercent(nextCpuPercent);
        setError(false);
      } catch {
        if (!disposed) setError(true);
      } finally {
        inFlight = false;
      }
    };

    void loadSnapshot();
    const timer = window.setInterval(() => {
      void loadSnapshot();
    }, 2500);

    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, []);

  const memoryPercent = getProcessMemoryBarPercent(snapshot?.memoryRssMb);
  const memorySummary = formatProcessMemory(snapshot?.memoryRssMb);
  const title = error
    ? tText('performance.unavailable')
    : tText('performance.title', {
      cpu: cpuPercent === undefined ? '--' : `${Math.round(cpuPercent)}%`,
      memory: memorySummary,
    });

  return (
    <div
      className={`titlebar-performance-monitor${error ? ' titlebar-performance-monitor-error' : ''}`}
      aria-label={title}
      title={title}
    >
      <span className="titlebar-performance-chip">
        <span className="titlebar-performance-label">CPU</span>
        <span className="titlebar-performance-track" aria-hidden="true">
          <span style={{ width: `${cpuPercent ?? 0}%` }} />
        </span>
        <span className="titlebar-performance-value">{cpuPercent === undefined ? '--' : `${Math.round(cpuPercent)}%`}</span>
      </span>
      <span className="titlebar-performance-chip">
        <span className="titlebar-performance-label">MEM</span>
        <span className="titlebar-performance-track" aria-hidden="true">
          <span style={{ width: `${memoryPercent}%` }} />
        </span>
        <span className="titlebar-performance-value">{memorySummary}</span>
      </span>
    </div>
  );
}
