use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{fence, AtomicU32, Ordering::*},
    usize,
};

use atomic_wait::{wait, wake_one};

const UNLOCKED: u32 = 0;
const LOCKED_UNCONTENDED: u32 = 1;
const LOCKED_CONTENDED: u32 = 2;

const MAX_SPINS: usize = 100;

pub struct Mutex<T> {
    /// 0: unlocked
    /// 1: locked, with no contention
    /// 2: locked, with other threads waiting
    state: AtomicU32,
    data: UnsafeCell<T>,
}

unsafe impl<T> Sync for Mutex<T> where T: Send {}

impl<T> Mutex<T> {
    pub const fn new(data: T) -> Self {
        Self {
            state: AtomicU32::new(UNLOCKED),
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> MutexGuard<T> {
        // Succeeds if
        // - either this is the only thread trying to lock. We may be able to
        //   avoid the `wake` syscall in this case, if no other thread tries to
        //   acquire the lock, before this thread unlocks.
        // - this is the winner among all the threads trying to lock. The other
        //   threads will mark the lock as contended, and this thread will know
        //   that it has to `wake` another thread.
        if self
            .state
            .compare_exchange(UNLOCKED, LOCKED_UNCONTENDED, Acquire, Relaxed)
            .is_err()
        {
            self.lock_contended();
        }
        MutexGuard { mutex: self }
    }

    // The `cold` attribute suggests that the function is unlikely to be called.
    #[cold]
    fn lock_contended(&self) {
        let mut spins = 0;
        // Spin only if this is the only thread waiting on it. (Else all the
        // waiting threads will be spinning!)
        while spins < MAX_SPINS && self.state.load(Relaxed) == LOCKED_UNCONTENDED {
            spins += 1;
            std::hint::spin_loop();
        }
        if self
            .state
            .compare_exchange(UNLOCKED, LOCKED_UNCONTENDED, Acquire, Relaxed)
            .is_ok()
        {
            return;
        }
        // Wait while the locking thread releases the lock.
        //
        // Also mark the lock as "contended" so that the locking thread
        // knows it has to wake this.
        while self.state.swap(LOCKED_CONTENDED, Relaxed) != UNLOCKED {
            wait(&self.state, LOCKED_CONTENDED);
        }
        fence(Acquire);
    }

    #[cfg(test)]
    fn is_locked(&self) -> bool {
        self.state.load(Relaxed) != UNLOCKED
    }

    fn unlock(&self) {
        // We can avoid the syscall if there's no thread waiting.
        if self.state.swap(UNLOCKED, Release) == LOCKED_CONTENDED {
            wake_one(&self.state);
        }
    }
}

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.unlock();
    }
}

#[cfg(test)]
mod test {
    use std::{
        sync::atomic::{AtomicU8, Ordering::*},
        thread,
        time::Duration,
    };

    use rand::random;

    use super::Mutex;

    #[test]
    fn lock_unlock() {
        let m = Mutex::new(123);
        assert!(!m.is_locked());
        let guard = m.lock();
        assert!(m.is_locked());
        drop(guard);
        assert!(!m.is_locked());
    }

    #[test]
    fn deref() {
        let m = Mutex::new(123);
        assert_eq!(*m.lock(), 123);
    }

    #[test]
    fn deref_mut() {
        let m = Mutex::new(123);
        let mut guard = m.lock();
        *guard = 456;
        drop(guard);
        assert_eq!(*m.lock(), 456);
    }

    #[test]
    fn sync() {
        for _ in 0..5 {
            let m = Mutex::new(vec![0]);
            let last = AtomicU8::new(0);
            thread::scope(|s| {
                s.spawn(|| {
                    // Randomize which thread acquires lock first
                    thread::sleep(Duration::from_millis(random::<u64>() % 50));

                    let mut guard = m.lock();
                    last.store(1, Relaxed);

                    thread::sleep(Duration::from_millis(10));
                    guard.push(1);
                    thread::sleep(Duration::from_millis(10));
                    guard.push(2);
                });

                s.spawn(|| {
                    // Randomize which thread acquires lock first
                    thread::sleep(Duration::from_millis(random::<u64>() % 50));
                    let mut guard = m.lock();
                    last.store(2, Relaxed);
                    thread::sleep(Duration::from_millis(10));
                    guard.push(3);
                    thread::sleep(Duration::from_millis(10));
                    guard.push(4);
                });
            });
            match last.load(Relaxed) {
                1 => assert_eq!(*m.lock(), vec![0, 3, 4, 1, 2]),
                2 => assert_eq!(*m.lock(), vec![0, 1, 2, 3, 4]),
                _ => unreachable!(),
            };
        }
    }
}
