//! One lazily created multi-thread tokio runtime that replaces `tauri::async_runtime`.
//!
//! The core crate uses reqwest/tokio, which need a tokio context; GPUI's executor has none.
//! `JoinHandle` futures are executor-agnostic, so they can be awaited inside `cx.spawn`.
use std::{future::Future, sync::OnceLock};
use tokio::{runtime::Runtime, task::JoinHandle};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// The shared runtime, created on first use.
pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("gtnh-backend")
            .build()
            .expect("failed to start the tokio runtime")
    })
}

/// Spawns a future on the runtime (fire-and-forget or await the handle).
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime().spawn(future)
}

/// Runs blocking work (filesystem, archives) on the runtime's blocking pool.
pub fn spawn_blocking<F, R>(work: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    runtime().spawn_blocking(work)
}

/// Spawns `future` immediately and returns a future (awaitable from GPUI) that yields its result;
/// a panicked or cancelled task becomes an `Err(String)`. The task keeps running if the returned
/// future is dropped.
pub fn run<T, F>(future: F) -> impl Future<Output = Result<T, String>>
where
    F: Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
{
    let handle = spawn(future);
    async move { handle.await.map_err(|e| e.to_string())? }
}

/// Like [`run`] for blocking closures.
pub fn run_blocking<T, F>(work: F) -> impl Future<Output = Result<T, String>>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    let handle = spawn_blocking(work);
    async move { handle.await.map_err(|e| e.to_string())? }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_run_on_the_runtime_and_flatten_errors() {
        let ok = futures::executor::block_on(run(async { Ok::<_, String>(2) }));
        assert_eq!(ok, Ok(2));
        let err = futures::executor::block_on(run_blocking(|| Err::<(), _>("bad".to_string())));
        assert_eq!(err, Err("bad".to_string()));
        let panicked =
            futures::executor::block_on(run_blocking(|| -> Result<(), String> { panic!("boom") }));
        assert!(panicked.is_err());
    }
}
