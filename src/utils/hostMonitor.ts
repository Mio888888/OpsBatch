export interface HostMonitorRefreshState {
  realtimeEnabled: boolean;
  hasSnapshot: boolean;
}

export function shouldPollHostMonitor(state: HostMonitorRefreshState) {
  return state.realtimeEnabled;
}

export function isHostMonitorIdle(state: HostMonitorRefreshState) {
  return !state.realtimeEnabled && !state.hasSnapshot;
}
