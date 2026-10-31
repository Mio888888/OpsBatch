//! Global log bridge (port of `utils/globalLogger.ts`): warnings/errors written through the `log` crate by our own
//! crates, panics and handled UI errors all reach the global log window (origin "frontend") and the diagnostics file.

use std::cell::Cell;
use std::sync::OnceLock;

use ops_core::Core;

static CORE: OnceLock<Core> = OnceLock::new();

thread_local! {
    /// Writing a log entry can itself log (database errors); never feed that back in.
    static IN_BRIDGE: Cell<bool> = const { Cell::new(false) };
}

/// Targets whose warnings/errors are ours (third-party chatter such as font warnings stays on stderr only).
pub fn is_own_target(target: &str) -> bool {
    target.starts_with("ops") || target.starts_with("opsbatch")
}

pub fn level_name(level: log::Level) -> &'static str {
    match level {
        log::Level::Error => "error",
        log::Level::Warn => "warn",
        _ => "info",
    }
}

/// `env_logger` plus forwarding of our warnings and errors.
pub struct BridgeLogger {
    inner: env_logger::Logger,
}

impl BridgeLogger {
    pub fn init(default_filter: &str) {
        let inner = env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or(default_filter),
        )
        .build();
        let max = inner.filter();
        if log::set_boxed_logger(Box::new(Self { inner })).is_ok() {
            // warnings/errors must pass the level gate even if RUST_LOG is stricter than the bridge needs
            log::set_max_level(max.max(log::LevelFilter::Warn));
        }
    }
}

impl log::Log for BridgeLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata)
            || (metadata.level() <= log::Level::Warn && is_own_target(metadata.target()))
    }

    fn log(&self, record: &log::Record) {
        if self.inner.matches(record) {
            self.inner.log(record);
        }
        if record.level() <= log::Level::Warn && is_own_target(record.target()) {
            if let Some(core) = CORE.get() {
                if !IN_BRIDGE.with(|f| f.replace(true)) {
                    ops_core::log::emit_frontend_log(
                        core,
                        level_name(record.level()),
                        record.target(),
                        &record.args().to_string(),
                    );
                    IN_BRIDGE.with(|f| f.set(false));
                }
            }
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// One line for a panic: message plus `file:line:col`.
pub fn panic_message(
    payload: &(dyn std::any::Any + Send),
    location: Option<&std::panic::Location>,
) -> String {
    let text = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".to_string());
    match location {
        Some(l) => format!("{text} ({}:{}:{})", l.file(), l.line(), l.column()),
        None => text,
    }
}

/// Attach the opened core: from now on bridged records and panics are stored.
pub fn attach(core: &Core) {
    let _ = CORE.set(core.clone());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        if let Some(core) = CORE.get() {
            let message = panic_message(info.payload(), info.location());
            if !IN_BRIDGE.with(|f| f.replace(true)) {
                ops_core::log::emit_frontend_log(core, "error", "panic", &message);
                IN_BRIDGE.with(|f| f.set(false));
            }
        }
    }));
}

/// A handled UI error (port of `logHandledError`).
pub fn log_handled_error(source: &str, error: &str, level: &str) {
    if let Some(core) = CORE.get() {
        ops_core::log::emit_frontend_log(core, level, source, error);
    }
}

#[cfg(test)]
mod tests {
    use super::{is_own_target, level_name, panic_message};

    #[test]
    fn only_our_crates_are_bridged() {
        assert!(is_own_target("opsbatch_app::sftp"));
        assert!(is_own_target("ops_ssh::ssh"));
        assert!(!is_own_target("gpui_apple::text_system"));
        assert_eq!(level_name(log::Level::Error), "error");
        assert_eq!(level_name(log::Level::Warn), "warn");
        assert_eq!(level_name(log::Level::Debug), "info");
    }

    #[test]
    fn panics_are_described_with_their_location() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_message(&*payload, None), "boom");
        let owned: Box<dyn std::any::Any + Send> = Box::new(String::from("kaput"));
        assert_eq!(panic_message(&*owned, None), "kaput");
        let other: Box<dyn std::any::Any + Send> = Box::new(5u8);
        assert_eq!(panic_message(&*other, None), "panic");
    }
}
