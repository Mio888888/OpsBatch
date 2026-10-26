//! Numbers behind the host monitor panel (port of the helpers and derived values in TerminalPage.tsx).

use std::collections::VecDeque;

use ops_ssh::hosts::{HostMonitorNetwork, HostMonitorSnapshot};

pub const HISTORY_LEN: usize = 28;

pub fn clamp_percent(v: f64) -> Option<f64> {
    v.is_finite().then(|| v.clamp(0.0, 100.0))
}

pub fn format_bytes_per_second(value: u64) -> String {
    if value >= 1024 * 1024 {
        format!("{:.1}M", value as f64 / 1024.0 / 1024.0)
    } else if value >= 1024 {
        format!("{}K", (value as f64 / 1024.0).round() as u64)
    } else {
        format!("{value}B")
    }
}

pub fn format_memory(used_mb: Option<u64>, total_mb: Option<u64>) -> String {
    match (used_mb, total_mb) {
        (Some(u), Some(t)) => format!("{:.1}G/{:.1}G", u as f64 / 1024.0, t as f64 / 1024.0),
        _ => "--".into(),
    }
}

/// Interfaces of a snapshot without duplicates (falls back to the single `network`).
pub fn snapshot_networks(s: &HostMonitorSnapshot) -> Vec<&HostMonitorNetwork> {
    let source: Vec<&HostMonitorNetwork> = if !s.networks.is_empty() { s.networks.iter().collect() } else { s.network.iter().collect() };
    let mut seen = std::collections::HashSet::new();
    source.into_iter().filter(|n| !n.interface.is_empty() && seen.insert(n.interface.clone())).collect()
}

fn network_by_interface<'a>(s: Option<&'a HostMonitorSnapshot>, name: &str) -> Option<&'a HostMonitorNetwork> {
    if name.is_empty() {
        return None;
    }
    snapshot_networks(s?).into_iter().find(|n| n.interface == name)
}

/// Bytes per second received/sent between two samples of the same interface.
pub fn network_rate(current: Option<&HostMonitorNetwork>, previous: Option<&HostMonitorNetwork>, elapsed_s: f64) -> (u64, u64) {
    match (current, previous) {
        (Some(c), Some(p)) if c.interface == p.interface && elapsed_s > 0.0 => (
            (c.rx_bytes.saturating_sub(p.rx_bytes) as f64 / elapsed_s).round() as u64,
            (c.tx_bytes.saturating_sub(p.tx_bytes) as f64 / elapsed_s).round() as u64,
        ),
        _ => (0, 0),
    }
}

#[derive(Default)]
pub struct MonitorModel {
    history: VecDeque<HostMonitorSnapshot>,
    selected_interface: String,
}

impl MonitorModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.history.clear();
        self.selected_interface.clear();
    }

    pub fn push(&mut self, snapshot: HostMonitorSnapshot) {
        self.history.push_back(snapshot);
        while self.history.len() > HISTORY_LEN {
            self.history.pop_front();
        }
        // Keep the selection valid: unknown/empty -> first interface.
        let active = self.active_interface();
        self.selected_interface = active;
    }

    pub fn len(&self) -> usize {
        self.history.len()
    }

    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }

    pub fn latest(&self) -> Option<&HostMonitorSnapshot> {
        self.history.back()
    }

    fn previous(&self) -> Option<&HostMonitorSnapshot> {
        (self.history.len() > 1).then(|| &self.history[self.history.len() - 2])
    }

    pub fn interfaces(&self) -> Vec<String> {
        self.latest().map(|s| snapshot_networks(s).into_iter().map(|n| n.interface.clone()).collect()).unwrap_or_default()
    }

    pub fn select_interface(&mut self, name: &str) {
        if self.interfaces().iter().any(|i| i == name) {
            self.selected_interface = name.to_string();
        }
    }

    /// The selected interface if it still exists, else the first one ("" when there is none).
    pub fn active_interface(&self) -> String {
        let all = self.interfaces();
        all.iter().find(|i| **i == self.selected_interface).or_else(|| all.first()).cloned().unwrap_or_default()
    }

    fn elapsed_s(a: &HostMonitorSnapshot, b: &HostMonitorSnapshot) -> f64 {
        (b.timestamp.saturating_sub(a.timestamp) as f64 / 1000.0).max(1.0)
    }

    /// (rx, tx) bytes/s of the active interface between the last two samples.
    pub fn current_rate(&self) -> (u64, u64) {
        let (Some(cur), Some(prev)) = (self.latest(), self.previous()) else { return (0, 0) };
        let name = self.active_interface();
        network_rate(network_by_interface(Some(cur), &name), network_by_interface(Some(prev), &name), Self::elapsed_s(prev, cur))
    }

    pub fn cpu_percent(&self) -> Option<f64> {
        let cur = self.latest()?;
        if let Some(p) = cur.cpu_percent {
            return clamp_percent(p);
        }
        let prev = self.previous()?;
        let (cu, ct, pu, pt) = (cur.cpu_time_used?, cur.cpu_time_total?, prev.cpu_time_used?, prev.cpu_time_total?);
        let d_total = ct as f64 - pt as f64;
        (d_total > 0.0).then(|| clamp_percent((cu as f64 - pu as f64) / d_total * 100.0)).flatten()
    }

    pub fn memory_percent(&self) -> Option<f64> {
        let s = self.latest()?;
        match (s.memory_used_mb, s.memory_total_mb) {
            (Some(u), Some(t)) if u > 0 && t > 0 => clamp_percent(u as f64 / t as f64 * 100.0),
            _ => None,
        }
    }

    pub fn swap_percent(&self) -> Option<f64> {
        let s = self.latest()?;
        match (s.swap_used_mb, s.swap_total_mb) {
            (Some(u), Some(t)) if u > 0 && t > 0 => clamp_percent(u as f64 / t as f64 * 100.0),
            _ => None,
        }
    }

    fn rate_at(&self, index: usize) -> (u64, u64) {
        if index == 0 {
            return (0, 0);
        }
        let name = self.active_interface();
        let (cur, prev) = (&self.history[index], &self.history[index - 1]);
        network_rate(network_by_interface(Some(cur), &name), network_by_interface(Some(prev), &name), Self::elapsed_s(prev, cur))
    }

    /// Bar heights (percent of the chart) of the receive rate for every sample.
    pub fn network_bars(&self) -> Vec<f32> {
        let max = (0..self.history.len()).map(|i| {
            let (rx, tx) = self.rate_at(i);
            rx.max(tx)
        }).max().unwrap_or(0).max(1);
        (0..self.history.len()).map(|i| (self.rate_at(i).0 as f32 / max as f32 * 100.0).clamp(6.0, 100.0)).collect()
    }

    pub fn ping_bars(&self) -> Vec<f32> {
        self.history.iter().map(|s| s.ping_ms.map(|p| (p as f32 * 3.0).clamp(3.0, 100.0)).unwrap_or(3.0)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(name: &str, rx: u64, tx: u64) -> HostMonitorNetwork {
        HostMonitorNetwork { interface: name.into(), rx_bytes: rx, tx_bytes: tx }
    }

    fn snap(ts: u64, networks: Vec<HostMonitorNetwork>) -> HostMonitorSnapshot {
        HostMonitorSnapshot {
            timestamp: ts,
            uptime: None,
            load_average: None,
            cpu_percent: None,
            cpu_time_used: None,
            cpu_time_total: None,
            memory_used_mb: None,
            memory_total_mb: None,
            swap_used_mb: None,
            swap_total_mb: None,
            os: None,
            kernel: None,
            processes: vec![],
            network: None,
            networks,
            ping_ms: None,
            filesystems: vec![],
        }
    }

    #[test]
    fn formatting_matches_the_original() {
        assert_eq!(format_bytes_per_second(512), "512B");
        assert_eq!(format_bytes_per_second(1536), "2K");
        assert_eq!(format_bytes_per_second(5 * 1024 * 1024 + 300 * 1024), "5.3M");
        assert_eq!(format_memory(Some(512), Some(2048)), "0.5G/2.0G");
        assert_eq!(format_memory(None, Some(2048)), "--");
        assert_eq!(clamp_percent(140.0), Some(100.0));
        assert_eq!(clamp_percent(f64::NAN), None);
    }

    #[test]
    fn networks_are_deduplicated_and_fall_back_to_single_network() {
        let mut s = snap(0, vec![net("eth0", 1, 1), net("eth0", 2, 2), net("", 3, 3), net("wlan0", 4, 4)]);
        assert_eq!(snapshot_networks(&s).iter().map(|n| n.interface.as_str()).collect::<Vec<_>>(), ["eth0", "wlan0"]);
        s.networks.clear();
        s.network = Some(net("ens3", 1, 1));
        assert_eq!(snapshot_networks(&s).len(), 1);
    }

    #[test]
    fn rate_needs_two_samples_of_the_same_interface() {
        let mut m = MonitorModel::new();
        m.push(snap(1_000, vec![net("eth0", 1000, 500)]));
        assert_eq!(m.current_rate(), (0, 0), "one sample is not enough");
        m.push(snap(4_000, vec![net("eth0", 7000, 2000)]));
        assert_eq!(m.current_rate(), (2000, 500));
        // Counter reset (reboot) must not underflow.
        m.push(snap(7_000, vec![net("eth0", 10, 10)]));
        assert_eq!(m.current_rate(), (0, 0));
    }

    #[test]
    fn selected_interface_survives_or_falls_back() {
        let mut m = MonitorModel::new();
        m.push(snap(0, vec![net("eth0", 0, 0), net("eth1", 0, 0)]));
        assert_eq!(m.active_interface(), "eth0");
        m.select_interface("eth1");
        assert_eq!(m.active_interface(), "eth1");
        m.select_interface("nope");
        assert_eq!(m.active_interface(), "eth1");
        m.push(snap(3_000, vec![net("eth0", 0, 0)]));
        assert_eq!(m.active_interface(), "eth0", "eth1 vanished");
    }

    #[test]
    fn cpu_uses_reported_percent_or_the_counter_delta() {
        let mut m = MonitorModel::new();
        let mut a = snap(0, vec![]);
        a.cpu_time_used = Some(100);
        a.cpu_time_total = Some(1000);
        m.push(a);
        assert_eq!(m.cpu_percent(), None);
        let mut b = snap(3_000, vec![]);
        b.cpu_time_used = Some(150);
        b.cpu_time_total = Some(1100);
        m.push(b);
        assert_eq!(m.cpu_percent(), Some(50.0));
        let mut c = snap(6_000, vec![]);
        c.cpu_percent = Some(12.5);
        m.push(c);
        assert_eq!(m.cpu_percent(), Some(12.5));
    }

    #[test]
    fn memory_percent_requires_nonzero_values() {
        let mut m = MonitorModel::new();
        let mut s = snap(0, vec![]);
        s.memory_used_mb = Some(512);
        s.memory_total_mb = Some(2048);
        s.swap_used_mb = Some(0);
        s.swap_total_mb = Some(1024);
        m.push(s);
        assert_eq!(m.memory_percent(), Some(25.0));
        assert_eq!(m.swap_percent(), None, "zero swap use shows no bar, like the original");
    }

    #[test]
    fn history_is_bounded_and_bars_are_clamped() {
        let mut m = MonitorModel::new();
        for i in 0..40u64 {
            m.push(snap(i * 3000, vec![net("eth0", i * 3000, i * 100)]));
        }
        assert_eq!(m.len(), HISTORY_LEN);
        let bars = m.network_bars();
        assert_eq!(bars.len(), HISTORY_LEN);
        assert!(bars.iter().all(|b| (6.0..=100.0).contains(b)));
        assert_eq!(*bars.last().unwrap(), 100.0);
        let mut p = snap(1, vec![]);
        p.ping_ms = Some(50.0);
        m.push(p);
        assert_eq!(*m.ping_bars().last().unwrap(), 100.0);
    }
}
