use std::sync::atomic::{AtomicU32, Ordering::*};

use atomic_wait::{wait, wake_one};

use crate::mutex::MutexGuard;

const NOT_READY: u32 = 0;
const READY: u32 = 1;

pub struct Condvar {
    state: AtomicU32,
}

impl Condvar {
    pub const fn new() -> Self {
        Self {
            state: AtomicU32::new(NOT_READY),
        }
    }

    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        let mutex = guard.mutex;
        drop(guard);
        while self.state.swap(NOT_READY, Relaxed) != READY {
            wait(&self.state, NOT_READY);
        }
        mutex.lock()
    }

    pub fn notify_one(&self) {
        self.state.store(READY, Relaxed);
        wake_one(&self.state);
    }
}

#[cfg(test)]
mod test {
    use std::{thread, time::Duration};

    use super::Condvar;
    use crate::mutex::Mutex;

    #[test]
    fn wait_non_zero() {
        let val = Mutex::new(0);
        let non_zero = Condvar::new();
        thread::scope(|s| {
            s.spawn(|| {
                let guard = val.lock();
                let guard = non_zero.wait(guard);
                assert_eq!(*guard, 1);
            });
            thread::sleep(Duration::from_millis(10));
            *val.lock() = 1;
            non_zero.notify_one();
        });
    }

    #[test]
    fn early_notify() {
        let val = Mutex::new(0);
        let non_zero = Condvar::new();
        thread::scope(|s| {
            *val.lock() = 1;
            non_zero.notify_one();
            s.spawn(|| {
                let guard = val.lock();
                let guard = non_zero.wait(guard);
                assert_eq!(*guard, 1);
            });
        });
    }
}
