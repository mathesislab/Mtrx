//! The tokio runtime the library runs on. The SDK needs tokio (timers,
//! sockets, spawned tasks), while Swift and Kotlin poll async calls on their
//! own executors; so each call runs as a task here and is awaited there.

use std::{
    future::Future,
    sync::{Arc, LazyLock},
};

use futures_util::StreamExt;
use mtrx_core::{AbortOnDrop, Live};
use tokio::runtime::{EnterGuard, Runtime};

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("mtrx")
        .build()
        .expect("could not start the tokio runtime")
});

/// Run `future` on the runtime and wait for it. If the caller stops
/// waiting (a cancelled Swift `Task` or Kotlin coroutine), it is cancelled.
pub(crate) async fn run<T: Send + 'static>(future: impl Future<Output = T> + Send + 'static) -> T {
    let mut task = AbortOnDrop(RUNTIME.spawn(future));
    match (&mut task.0).await {
        Ok(value) => value,
        // Never cancelled while awaited: only dropping `task` aborts it.
        Err(error) => std::panic::resume_unwind(error.into_panic()),
    }
}

/// Enter the runtime until the guard is dropped: for calls that are not
/// async but start tasks.
pub(crate) fn enter() -> EnterGuard<'static> {
    RUNTIME.enter()
}

/// Keeps `T` alive and drops it inside the runtime: Swift and Kotlin
/// release objects on their own threads, and the SDK's types may need the
/// runtime when they are dropped.
pub(crate) struct InRuntime<T>(Option<T>);

impl<T> InRuntime<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(Some(value))
    }

    pub(crate) fn into_inner(mut self) -> T {
        self.0.take().expect("present until dropped")
    }
}

impl<T> std::ops::Deref for InRuntime<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0.as_ref().expect("present until dropped")
    }
}

impl<T> Drop for InRuntime<T> {
    fn drop(&mut self) {
        let _runtime = RUNTIME.enter();
        drop(self.0.take());
    }
}

/// A subscription (to a listener), running until cancelled. Dropping the
/// handle cancels it too: Swift releases it with its last reference; in
/// Kotlin call `cancel()` or `close()`, as the garbage collector may take a
/// while.
#[derive(uniffi::Object)]
pub struct TaskHandle(AbortOnDrop);

impl TaskHandle {
    pub(crate) fn spawn(task: impl Future<Output = ()> + Send + 'static) -> Arc<Self> {
        Arc::new(Self(AbortOnDrop(RUNTIME.spawn(task))))
    }
}

/// A subscription calling `on_update` with `live`'s value as `map` makes
/// it, now and after every change, until cancelled or the value's task
/// ends.
pub(crate) fn follow<T, R: 'static>(
    live: Live<T>,
    map: fn(T) -> R,
    on_update: impl Fn(R) + Send + 'static,
) -> Arc<TaskHandle>
where
    T: Clone + Send + Sync + 'static,
{
    TaskHandle::spawn(forward(live, map, on_update))
}

/// What [`follow`] runs, for a subscription that first has to get the
/// value.
pub(crate) async fn forward<T, R>(live: Live<T>, map: fn(T) -> R, on_update: impl Fn(R))
where
    T: Clone + Send + Sync + 'static,
{
    let mut updates = std::pin::pin!(live.updates());
    while let Some(value) = updates.next().await {
        on_update(map(value));
    }
}

#[uniffi::export]
impl TaskHandle {
    /// Stop the subscription; its listener is not called afterwards.
    pub fn cancel(&self) {
        self.0.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use super::*;

    /// Awaited without a tokio runtime, as Swift and Kotlin do.
    fn block_on<T>(future: impl Future<Output = T>) -> T {
        futures_executor::block_on(future)
    }

    #[test]
    fn calls_run_on_the_runtime() {
        let value = block_on(run(async {
            tokio::time::sleep(Duration::from_millis(1)).await;
            tokio::runtime::Handle::current();
            42
        }));
        assert_eq!(value, 42);
    }

    #[test]
    fn dropping_a_call_cancels_it() {
        let finished = Arc::new(AtomicBool::new(false));
        let call = run({
            let finished = finished.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                finished.store(true, Ordering::SeqCst);
            }
        });
        // Polled once, so the task is spawned, then dropped.
        block_on(async {
            let mut call = std::pin::pin!(call);
            assert!(futures_util::poll!(&mut call).is_pending());
        });
        std::thread::sleep(Duration::from_millis(100));
        assert!(!finished.load(Ordering::SeqCst));
    }

    #[test]
    fn task_handles_cancel_when_dropped() {
        let handle = TaskHandle::spawn(std::future::pending());
        let finished = |handle: &TaskHandle| handle.0.0.is_finished();
        assert!(!finished(&handle));
        handle.cancel();
        for _ in 0..100 {
            if finished(&handle) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(finished(&handle));
    }

    #[test]
    #[should_panic(expected = "boom")]
    fn panics_reach_the_caller() {
        block_on(run(async { panic!("boom") }))
    }
}
