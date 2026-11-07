//! Tasks with an owner: loads end with what asked for them, requests run
//! to their end whoever asked (`docs/app-structure.md`, item 6).

use std::{
    collections::HashMap,
    hash::Hash,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use iced::{Task, task::Handle};

/// Loads a component owns: aborted when it is dropped, or when one with
/// the same key starts. A load ends with its result.
pub struct Owned<K: Hash + Eq = &'static str>(HashMap<K, Load>);

/// A load started, and whether its result came.
struct Load {
    /// Aborts it when dropped.
    _handle: Handle,
    done: Arc<AtomicBool>,
}

impl<K: Hash + Eq> Default for Owned<K> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

impl<K: Hash + Eq> Owned<K> {
    /// Own `task` under `key`, aborting the one there.
    pub fn run<M: Send + 'static>(&mut self, key: K, task: Task<M>) -> Task<M> {
        // Nothing to do, so nothing running.
        if task.units() == 0 {
            self.0.remove(&key);
            return task;
        }
        let done = Arc::new(AtomicBool::new(false));
        // Set as the result comes, before the app sees it.
        let (task, handle) = task
            .map({
                let done = done.clone();
                move |result| {
                    done.store(true, Ordering::Relaxed);
                    result
                }
            })
            .abortable();
        let load = Load {
            _handle: handle.abort_on_drop(),
            done,
        };
        self.0.insert(key, load);
        task
    }

    /// Own `work`'s result under `key`, its error as text: see [`request`].
    pub fn load<T, M>(
        &mut self,
        key: K,
        work: impl Future<Output = mtrx_core::Result<T>> + Send + 'static,
        done: impl FnOnce(Result<T, String>) -> M + Send + 'static,
    ) -> Task<M>
    where
        T: Send + 'static,
        M: Send + 'static,
    {
        self.run(key, request(work, done))
    }

    pub fn cancel(&mut self, key: &K) {
        self.0.remove(key);
    }

    /// The load under `key` was started and its result has not come.
    pub fn running(&self, key: &K) -> bool {
        self.0
            .get(key)
            .is_some_and(|load| !load.done.load(Ordering::Relaxed))
    }
}

/// A request to the server: runs to its end whoever asked; its error as
/// text.
pub fn request<T, M>(
    work: impl Future<Output = mtrx_core::Result<T>> + Send + 'static,
    done: impl FnOnce(Result<T, String>) -> M + Send + 'static,
) -> Task<M>
where
    T: Send + 'static,
    M: Send + 'static,
{
    Task::perform(work, move |result| done(result.map_err(|e| e.to_string())))
}

/// No result wanted: a failure is logged, not dropped. `what` failed, as in
/// "could not {what}".
pub fn background<M: Send + 'static>(
    what: &'static str,
    work: impl Future<Output = mtrx_core::Result<()>> + Send + 'static,
) -> Task<M> {
    Task::future(async move {
        if let Err(e) = work.await {
            tracing::warn!("could not {what}: {e}");
        }
    })
    .discard()
}

/// Run `task` to its end, as iced would, for what it gives.
#[cfg(test)]
pub async fn outputs<M: Send + 'static>(task: Task<M>) -> Vec<M> {
    use futures_util::StreamExt;
    let Some(stream) = iced_runtime::task::into_stream(task) else {
        return Vec::new();
    };
    stream
        .filter_map(|action| async move {
            match action {
                iced_runtime::Action::Output(output) => Some(output),
                _ => None,
            }
        })
        .collect()
        .await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn after(ms: u64, result: u32) -> Task<u32> {
        Task::perform(tokio::time::sleep(Duration::from_millis(ms)), move |()| {
            result
        })
    }

    #[tokio::test]
    async fn a_load_runs_until_its_result_comes() {
        let mut owned = Owned::default();
        let load = owned.run("members", after(10, 1));
        assert!(owned.running(&"members"));
        assert!(!owned.running(&"pins"));
        assert_eq!(outputs(load).await, [1]);
        assert!(!owned.running(&"members"), "done once its result came");
    }

    #[tokio::test]
    async fn a_load_ends_with_its_owner() {
        let mut owned = Owned::default();
        let load = owned.run("members", after(10, 1));
        drop(owned);
        assert!(outputs(load).await.is_empty());
    }

    #[tokio::test]
    async fn a_load_is_replaced_by_one_with_its_key_and_can_be_cancelled() {
        let mut owned = Owned::default();
        let first = owned.run("seek", after(10, 1));
        let second = owned.run("seek", after(10, 2));
        let other = owned.run("members", after(10, 3));
        owned.cancel(&"members");
        assert!(owned.running(&"seek"));
        assert!(!owned.running(&"members"));
        assert!(outputs(first).await.is_empty());
        assert_eq!(outputs(second).await, [2]);
        assert!(outputs(other).await.is_empty());
    }

    #[tokio::test]
    async fn nothing_to_do_is_not_running() {
        let mut owned = Owned::default();
        let _ = owned.run("seek", after(10, 1));
        let _ = owned.run("seek", Task::<u32>::none());
        assert!(!owned.running(&"seek"));
    }

    #[tokio::test]
    async fn a_request_gives_its_error_as_text() {
        let failed = request(
            async { Err::<(), _>(mtrx_core::Error::EventNotFound) },
            |result| result,
        );
        let [Err(e)] = &outputs(failed).await[..] else {
            panic!("one failure");
        };
        assert_eq!(*e, mtrx_core::Error::EventNotFound.to_string());
    }
}
