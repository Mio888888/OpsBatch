/// Host monitor refresh rules: nothing polls until the user presses "start realtime monitor".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostMonitorRefreshState {
    pub realtime_enabled: bool,
    pub has_snapshot: bool,
}

pub fn should_poll_host_monitor(state: HostMonitorRefreshState) -> bool {
    state.realtime_enabled
}

pub fn is_host_monitor_idle(state: HostMonitorRefreshState) -> bool {
    !state.realtime_enabled && !state.has_snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_is_disabled_by_default() {
        let s = HostMonitorRefreshState {
            realtime_enabled: false,
            has_snapshot: false,
        };
        assert!(!should_poll_host_monitor(s));
        assert!(is_host_monitor_idle(s));
    }

    #[test]
    fn polling_starts_after_realtime_is_enabled() {
        let s = HostMonitorRefreshState {
            realtime_enabled: true,
            has_snapshot: false,
        };
        assert!(should_poll_host_monitor(s));
        assert!(!is_host_monitor_idle(s));
    }

    #[test]
    fn a_stopped_monitor_with_a_snapshot_is_not_idle() {
        assert!(!is_host_monitor_idle(HostMonitorRefreshState {
            realtime_enabled: false,
            has_snapshot: true
        }));
    }
}
