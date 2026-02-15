use std::sync::{Condvar, Mutex, OnceLock};

#[derive(Debug)]
struct AiLimiterInner {
    max_permits: usize,
    in_use: usize,
}

#[derive(Debug)]
struct AiLimiter {
    inner: Mutex<AiLimiterInner>,
    cv: Condvar,
}

static AI_LIMITER: OnceLock<AiLimiter> = OnceLock::new();

fn limiter() -> &'static AiLimiter {
    AI_LIMITER.get_or_init(|| AiLimiter {
        inner: Mutex::new(AiLimiterInner {
            max_permits: 32,
            in_use: 0,
        }),
        cv: Condvar::new(),
    })
}

pub(crate) struct AiPermit {
    _private: (),
}

impl Drop for AiPermit {
    fn drop(&mut self) {
        let lim = limiter();
        let mut guard = lim
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.in_use = guard.in_use.saturating_sub(1);
        lim.cv.notify_one();
    }
}

pub(crate) fn set_max_permits(max_permits: usize) {
    let lim = limiter();
    let mut guard = lim
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.max_permits = max_permits.max(1);
    lim.cv.notify_all();
}

pub(crate) fn acquire_permit() -> AiPermit {
    let lim = limiter();
    let mut guard = lim
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    while guard.in_use >= guard.max_permits {
        guard = lim
            .cv
            .wait(guard)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }

    guard.in_use = guard.in_use.saturating_add(1);
    AiPermit { _private: () }
}
