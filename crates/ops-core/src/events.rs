//! In-process replacement for Tauri `emit`/`listen`.
//!
//! * `EventBus`: topic events (`exec:{id}:output`, `global-log`, ...). Subscribers get an unbounded channel, so
//!   nothing is ever dropped (batch execution results must not be lossy). Dead subscribers are pruned lazily.
//! * `TerminalStreams`: one lossless channel per terminal session; the receiver is created together with the
//!   session and claimed by the view later, so output emitted before the view exists is not lost.

use std::sync::Arc;

use dashmap::DashMap;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;

/// Same shape as a Tauri event: a topic string and a JSON payload.
#[derive(Debug, Clone)]
pub struct AppEvent {
    pub topic: String,
    pub payload: Value,
}

impl AppEvent {
    pub fn new(topic: impl Into<String>, payload: impl Serialize) -> Self {
        Self {
            topic: topic.into(),
            payload: serde_json::to_value(payload).unwrap_or(Value::Null),
        }
    }

    /// Deserialize the payload.
    pub fn parse<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_value(self.payload.clone()).ok()
    }
}

struct Subscriber {
    /// Exact topic, or prefix when it ends with `*` (e.g. `exec:*`).
    pattern: String,
    tx: flume::Sender<Arc<AppEvent>>,
}

impl Subscriber {
    fn matches(&self, topic: &str) -> bool {
        match self.pattern.strip_suffix('*') {
            Some(prefix) => topic.starts_with(prefix),
            None => self.pattern == topic,
        }
    }
}

#[derive(Clone, Default)]
pub struct EventBus {
    subs: Arc<Mutex<Vec<Subscriber>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&self, topic: impl Into<String>, payload: impl Serialize) {
        let event = Arc::new(AppEvent::new(topic, payload));
        let mut subs = self.subs.lock();
        subs.retain(|s| {
            if !s.matches(&event.topic) {
                return true;
            }
            s.tx.send(event.clone()).is_ok()
        });
    }

    /// Subscribe to an exact topic, or a prefix with a trailing `*`.
    pub fn subscribe(&self, pattern: impl Into<String>) -> flume::Receiver<Arc<AppEvent>> {
        let (tx, rx) = flume::unbounded();
        self.subs.lock().push(Subscriber { pattern: pattern.into(), tx });
        rx
    }
}

/// Terminal session output. `""` (empty string) means the session is closed, like the Tauri event did.
#[derive(Clone, Default)]
pub struct TerminalStreams {
    senders: Arc<DashMap<String, flume::Sender<String>>>,
    receivers: Arc<DashMap<String, flume::Receiver<String>>>,
}

impl TerminalStreams {
    pub fn new() -> Self {
        Self::default()
    }

    /// Called by the backend when a session is created.
    pub fn open(&self, session_id: &str) {
        if self.senders.contains_key(session_id) {
            return; // idempotent: the backend may open eagerly and again from the output pump
        }
        let (tx, rx) = flume::unbounded();
        self.senders.insert(session_id.to_string(), tx);
        self.receivers.insert(session_id.to_string(), rx);
    }

    pub fn emit(&self, session_id: &str, data: String) {
        if let Some(tx) = self.senders.get(session_id) {
            let _ = tx.send(data);
        }
    }

    /// Called by the view; each session can be claimed once.
    pub fn take(&self, session_id: &str) -> Option<flume::Receiver<String>> {
        self.receivers.remove(session_id).map(|(_, rx)| rx)
    }

    /// Forget a stream nobody will read (failed connects).
    pub fn discard(&self, session_id: &str) {
        self.senders.remove(session_id);
        self.receivers.remove(session_id);
    }

    pub fn close(&self, session_id: &str) {
        self.emit(session_id, String::new());
        self.senders.remove(session_id);
        // An unclaimed receiver is kept until taken so the close marker and buffered output stay readable.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_and_prefix_subscriptions() {
        let bus = EventBus::new();
        let exact = bus.subscribe("exec:t1:done");
        let prefix = bus.subscribe("exec:*");
        let other = bus.subscribe("global-log");
        bus.publish("exec:t1:output", json!({"a": 1}));
        bus.publish("exec:t1:done", json!({"ok": true}));
        assert_eq!(exact.try_iter().count(), 1);
        assert_eq!(prefix.try_iter().count(), 2);
        assert_eq!(other.try_iter().count(), 0);
    }

    #[test]
    fn dead_subscribers_are_pruned_and_nothing_is_lost() {
        let bus = EventBus::new();
        let rx = bus.subscribe("t");
        drop(bus.subscribe("t"));
        for i in 0..10_000 {
            bus.publish("t", i);
        }
        assert_eq!(rx.try_iter().count(), 10_000);
        assert_eq!(bus.subs.lock().len(), 1);
    }

    #[test]
    fn terminal_open_is_idempotent() {
        let ts = TerminalStreams::new();
        ts.open("s");
        ts.emit("s", "a".into());
        ts.open("s");
        ts.emit("s", "b".into());
        let got: Vec<String> = ts.take("s").unwrap().try_iter().collect();
        assert_eq!(got, vec!["a", "b"]);
    }

    #[test]
    fn terminal_output_before_view_is_buffered() {
        let ts = TerminalStreams::new();
        ts.open("s1");
        ts.emit("s1", "hello".into());
        ts.emit("s1", "world".into());
        ts.close("s1");
        let rx = ts.take("s1").expect("claim once");
        let got: Vec<String> = rx.try_iter().collect();
        assert_eq!(got, vec!["hello", "world", ""]);
        assert!(ts.take("s1").is_none());
    }
}
