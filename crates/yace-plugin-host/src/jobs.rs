//! Background jobs for plugins (spec §18.10): a small pool of worker
//! threads, created on first use, with a cancellation flag per job. Jobs of
//! a plugin are cancelled when it deactivates.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

type Task = Box<dyn FnOnce() + Send>;

/// Worker threads for plugin jobs.
pub const WORKERS: usize = 2;

#[derive(Default)]
struct Inner {
    next: AtomicU64,
    /// Job id → (plugin index, cancelled flag).
    flags: Mutex<HashMap<u64, (usize, Arc<AtomicBool>)>>,
    queue: Mutex<Option<mpsc::Sender<Task>>>,
}

#[derive(Clone, Default)]
pub struct Jobs {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Jobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Jobs({} running)", self.inner.flags.lock().map(|f| f.len()).unwrap_or(0))
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Jobs {
    fn sender(&self) -> mpsc::Sender<Task> {
        let mut q = lock(&self.inner.queue);
        if let Some(tx) = q.as_ref() {
            return tx.clone();
        }
        let (tx, rx) = mpsc::channel::<Task>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..WORKERS {
            let rx = Arc::clone(&rx);
            let _ = thread::Builder::new().name(format!("yace-plugin-job-{i}")).spawn(move || {
                loop {
                    let task = lock(&rx).recv();
                    match task {
                        Ok(t) => {
                            // A panicking job must not kill the worker.
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(t));
                        }
                        Err(_) => return,
                    }
                }
            });
        }
        *q = Some(tx.clone());
        tx
    }

    /// Queue `f(job_id)` for `plugin`. Returns the job id.
    pub fn spawn(&self, plugin: usize, f: impl FnOnce(u64) + Send + 'static) -> u64 {
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed) + 1;
        lock(&self.inner.flags).insert(id, (plugin, Arc::new(AtomicBool::new(false))));
        let inner = Arc::clone(&self.inner);
        let _ = self.sender().send(Box::new(move || {
            f(id);
            lock(&inner.flags).remove(&id);
        }));
        id
    }

    /// True if the job was cancelled, finished or never existed.
    pub fn cancelled(&self, id: u64) -> bool {
        lock(&self.inner.flags).get(&id).is_none_or(|(_, f)| f.load(Ordering::Relaxed))
    }

    /// Cancel every job of a plugin.
    pub fn cancel_plugin(&self, plugin: usize) {
        for (p, f) in lock(&self.inner.flags).values() {
            if *p == plugin {
                f.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Jobs of a plugin still queued or running.
    pub fn running(&self, plugin: usize) -> usize {
        lock(&self.inner.flags).values().filter(|(p, _)| *p == plugin).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn jobs_run_and_cancel() {
        let jobs = Jobs::default();
        let (tx, rx) = mpsc::channel();
        let j = jobs.clone();
        let id = jobs.spawn(7, move |id| {
            let start = Instant::now();
            while !j.cancelled(id) && start.elapsed() < Duration::from_secs(5) {
                thread::sleep(Duration::from_millis(2));
            }
            tx.send(j.cancelled(id)).unwrap();
        });
        assert!(!jobs.cancelled(id));
        assert_eq!(jobs.running(7), 1);
        jobs.cancel_plugin(7);
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "the job saw the cancellation");
        let start = Instant::now();
        while jobs.running(7) > 0 && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(jobs.running(7), 0);
        assert!(jobs.cancelled(id), "finished jobs count as cancelled");
        // A panicking job doesn't stop the pool.
        jobs.spawn(1, |_| panic!("boom"));
        let (tx, rx) = mpsc::channel();
        jobs.spawn(1, move |_| tx.send(42).unwrap());
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 42);
    }
}
