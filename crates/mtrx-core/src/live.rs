//! Values that change: a value now, then each change, kept up to date by a
//! task while anyone holds it; and tasks stopped once nobody holds them.

use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_util::{Stream, StreamExt};
use tokio::{sync::watch, task::JoinHandle};
use tokio_stream::wrappers::WatchStream;

/// A spawned task, aborted when dropped.
pub struct AbortOnDrop<T = ()>(pub JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A value kept up to date by a task, while any clone is alive.
///
/// A value an object keeps up to date (a timeline's rows, the connection)
/// ends with the object: its task is the object's.
#[derive(Clone)]
pub struct Live<T> {
    value: watch::Receiver<T>,
    _task: Option<Arc<AbortOnDrop>>,
}

impl<T: Clone + Send + Sync + 'static> Live<T> {
    /// The value now.
    pub fn get(&self) -> T {
        self.value.borrow().clone()
    }

    /// The value now, then each change; ends with the task. Keeps the task
    /// running while it is read.
    pub fn updates(&self) -> impl Stream<Item = T> + use<T> {
        Updates {
            values: WatchStream::new(self.value.clone()),
            _task: self._task.clone(),
        }
    }

    /// The first value, from now, for which `done` is true; the last one
    /// if the task ended before.
    pub async fn wait_for(&self, done: impl Fn(&T) -> bool) -> T {
        let mut value = self.value.clone();
        match value.wait_for(done).await {
            Ok(value) => value.clone(),
            Err(_) => self.get(),
        }
    }

    /// `initial`, then the values `run` sends, in a task that runs while
    /// any clone or [`Live::updates`] stream is alive.
    pub(crate) fn spawn<F>(initial: T, run: impl FnOnce(watch::Sender<T>) -> F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let (tx, value) = watch::channel(initial);
        Self {
            value,
            _task: Some(Arc::new(AbortOnDrop(tokio::spawn(run(tx))))),
        }
    }

    /// The values sent on `rx`'s channel; ends when its sender is dropped.
    pub(crate) fn from_watch(rx: watch::Receiver<T>) -> Self {
        Self {
            value: rx,
            _task: None,
        }
    }
}

/// [`Live::updates`].
struct Updates<T> {
    values: WatchStream<T>,
    _task: Option<Arc<AbortOnDrop>>,
}

impl<T: Clone + Send + Sync + 'static> Stream for Updates<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        self.values.poll_next_unpin(cx)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A value that counts up from 0 to `until`, one a millisecond; and a
    /// receiver whose sender lives as long as the task.
    fn counting(until: u32) -> (Live<u32>, watch::Receiver<()>) {
        let (alive, running) = watch::channel(());
        let live = Live::spawn(0, move |count| async move {
            let _alive = alive;
            for n in 1..=until {
                tokio::time::sleep(Duration::from_millis(1)).await;
                count.send_replace(n);
            }
        });
        (live, running)
    }

    /// Until the task holding `running`'s sender ends.
    async fn ended(mut running: watch::Receiver<()>) {
        while running.changed().await.is_ok() {}
    }

    #[tokio::test]
    async fn updates_give_the_value_now_then_each_change_and_end_with_the_task() {
        let (live, _running) = counting(3);
        let seen: Vec<u32> = live.updates().collect().await;
        assert_eq!(seen.first(), Some(&0));
        assert_eq!(seen.last(), Some(&3));
        assert!(seen.is_sorted());
        assert_eq!(live.get(), 3);
    }

    #[tokio::test]
    async fn the_task_stops_with_the_last_clone() {
        let (live, running) = counting(u32::MAX);
        let clone = live.clone();
        drop(live);
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(running.has_changed().is_ok(), "stopped with a clone alive");
        drop(clone);
        tokio::time::timeout(Duration::from_secs(1), ended(running))
            .await
            .expect("the task stops");
    }

    #[tokio::test]
    async fn an_updates_stream_keeps_the_task_running() {
        let (live, _running) = counting(3);
        let updates = live.updates();
        drop(live);
        let seen: Vec<u32> = updates.collect().await;
        assert_eq!(seen.last(), Some(&3));
    }

    #[tokio::test]
    async fn waiting_for_a_value() {
        let (live, _running) = counting(10);
        assert!(live.wait_for(|n| *n >= 5).await >= 5);
        // Never true: the last value, once the task ended.
        assert_eq!(live.wait_for(|n| *n > 10).await, 10);
    }

    #[tokio::test]
    async fn a_watched_value_ends_with_its_sender() {
        let (tx, rx) = watch::channel(1);
        let live = Live::from_watch(rx);
        let updates = live.updates();
        tx.send_replace(2);
        drop(tx);
        assert_eq!(updates.collect::<Vec<_>>().await, [2]);
        assert_eq!(live.get(), 2);
    }

    #[tokio::test]
    async fn tasks_abort_on_drop() {
        let (alive, running) = watch::channel(());
        let task = AbortOnDrop(tokio::spawn(async move {
            let _alive = alive;
            std::future::pending::<()>().await;
        }));
        drop(task);
        tokio::time::timeout(Duration::from_secs(1), ended(running))
            .await
            .expect("the task stops");
    }
}
