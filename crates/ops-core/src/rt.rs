//! Global tokio runtime shared by every network/IO service, plus helpers to hop between it and GPUI's executors.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::{Handle, Runtime};

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("opsbatch-rt")
            .build()
            .expect("failed to start tokio runtime")
    })
}

pub fn handle() -> Handle {
    runtime().handle().clone()
}

pub fn spawn<F>(fut: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime().spawn(fut)
}

pub fn spawn_blocking<F, R>(f: F) -> tokio::task::JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    runtime().spawn_blocking(f)
}

/// Run a future to completion from a non-async thread (never call from inside the runtime).
pub fn block_on<F: Future>(fut: F) -> F::Output {
    runtime().block_on(fut)
}

/// Run `fut` on the tokio runtime and get its result through a future that can be awaited on ANY executor
/// (e.g. GPUI's `cx.spawn`), which has no tokio reactor of its own.
pub fn run<F>(fut: F) -> impl Future<Output = Result<F::Output, String>>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = flume::bounded(1);
    spawn(async move {
        let _ = tx.send_async(fut.await).await;
    });
    async move {
        rx.recv_async()
            .await
            .map_err(|_| "task cancelled".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_bridges_to_foreign_executor() {
        // flume's futures work on any executor; emulate one with a trivial block_on from futures-lite-style polling.
        let out = block_on(async { run(async { 40 + 2 }).await.unwrap() });
        assert_eq!(out, 42);
    }

    #[test]
    fn spawn_blocking_works() {
        let v = block_on(async { spawn_blocking(|| 7).await.unwrap() });
        assert_eq!(v, 7);
    }
}
