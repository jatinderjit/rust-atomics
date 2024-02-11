use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::*};

use atomic_wait::{wait, wake_all, wake_one};

use crate::mutex::MutexGuard;

pub struct Condvar {
    counter: AtomicU32,
    num_waiters: AtomicUsize,
}

impl Condvar {
    pub const fn new() -> Self {
        Self {
            counter: AtomicU32::new(0),
            num_waiters: AtomicUsize::new(0),
        }
    }

    /// Wait allows spurious wake-ups
    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        self.num_waiters.fetch_add(1, Relaxed);

        // Load the value before unlocking for an "atomic" behaviour.
        // This prevents losing a signal between unlocking the mutex and reading
        // the value (if we read this after unlocking).
        let counter = self.counter.load(Relaxed);

        let mutex = guard.mutex;
        drop(guard);

        // Wait only if the counter hasn't changed since unlocking.
        wait(&self.counter, counter);

        self.num_waiters.fetch_sub(1, Relaxed);

        mutex.lock()
    }

    // notify_one can actually "wake" more than one thread!
    // If there's a thread that has loaded a counter just before notify_one,
    // then it won't `wait`. Meanwhile, another sleeping thread will be woken up
    // by `wake_one`, and both will be racing for the mutex.
    pub fn notify_one(&self) {
        if self.num_waiters.load(Relaxed) > 0 {
            self.counter.fetch_add(1, Relaxed);
            wake_one(&self.counter);
        }
    }

    pub fn notify_all(&self) {
        if self.num_waiters.load(Relaxed) > 0 {
            self.counter.fetch_add(1, Relaxed);
            wake_all(&self.counter);
        }
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
                // Flaky, since we don't handle spurious wake-ups.
                let guard = non_zero.wait(guard);
                assert_eq!(*guard, 1);
            });
            thread::sleep(Duration::from_millis(10));
            *val.lock() = 1;
            non_zero.notify_one();
        });
    }

    #[test]
    fn test_allow_spurious() {
        // Copied test case
        // https://github.com/m-ou-se/rust-atomics-and-locks/blob/main/src/ch9_locks/condvar_1.rs
        let mutex = Mutex::new(0);
        let condvar = Condvar::new();

        let mut wakeups = 0;

        thread::scope(|s| {
            s.spawn(|| {
                thread::sleep(Duration::from_secs(1));
                *mutex.lock() = 123;
                condvar.notify_one();
            });

            let mut m = mutex.lock();
            while *m < 100 {
                m = condvar.wait(m);
                wakeups += 1;
            }

            assert_eq!(*m, 123);
        });

        // Check that the main thread actually did wait (not busy-loop),
        // while still allowing for a few spurious wake ups.
        assert!(wakeups < 10);
    }
}
