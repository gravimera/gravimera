use std::sync::{Arc, Mutex};

pub(crate) type SharedResult<T, E> = Arc<Mutex<Option<Result<T, E>>>>;

pub(crate) fn new_shared_result<T, E>() -> SharedResult<T, E> {
    Arc::new(Mutex::new(None))
}

pub(crate) fn take_shared_result<T, E>(shared: &SharedResult<T, E>) -> Option<Result<T, E>> {
    let Ok(mut guard) = shared.lock() else {
        return None;
    };
    guard.take()
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SharedResultSetMetrics {
    pub(crate) lock_wait_ms: u128,
    pub(crate) poisoned: bool,
}

pub(crate) fn set_shared_result_with_metrics<T, E>(
    shared: &SharedResult<T, E>,
    value: Result<T, E>,
) -> SharedResultSetMetrics {
    let started_at = std::time::Instant::now();
    let (mut guard, poisoned) = match shared.lock() {
        Ok(guard) => (guard, false),
        Err(poisoned) => (poisoned.into_inner(), true),
    };
    let lock_wait_ms = started_at.elapsed().as_millis();
    *guard = Some(value);
    SharedResultSetMetrics {
        lock_wait_ms,
        poisoned,
    }
}

pub(crate) fn spawn_worker_thread<T, E, F, G>(
    thread_name: String,
    shared: SharedResult<T, E>,
    work: F,
    on_store: G,
) -> std::io::Result<std::thread::JoinHandle<()>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: FnOnce() -> Result<T, E> + Send + 'static,
    G: FnOnce(SharedResultSetMetrics) + Send + 'static,
{
    std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let result = work();
            let metrics = set_shared_result_with_metrics(&shared, result);
            on_store(metrics);
        })
}
