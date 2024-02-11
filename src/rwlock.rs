use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{fence, AtomicU32, Ordering::*},
};

use atomic_wait::{wait, wake_all, wake_one};

const UNLOCKED: u32 = 0;
const READ_LOCK: u32 = 1;
const WRITE_LOCK: u32 = 2;

pub struct RwLock<T> {
    readers: AtomicU32,
    state: AtomicU32,
    data: UnsafeCell<T>,
}

// In addition to the `Send` constraint like in `Mutex`, we additionally need
// the `Sync` constraint since multiple readers can access the data.
unsafe impl<T> Sync for RwLock<T> where T: Send + Sync {}

impl<T> RwLock<T> {
    pub fn new(data: T) -> Self {
        Self {
            readers: AtomicU32::new(0),
            state: AtomicU32::new(UNLOCKED),
            data: UnsafeCell::new(data),
        }
    }

    pub fn read(&self) -> RLockGuard<T> {
        let mut readers = self.readers.load(Relaxed);
        loop {
            match self
                .readers
                .compare_exchange(readers, readers + 1, Relaxed, Relaxed)
            {
                Ok(_) => break,
                Err(val) => {
                    readers = val;
                    while readers == u32::MAX {
                        readers = self.readers.load(Relaxed);
                        std::hint::spin_loop();
                    }
                }
            };
        }
        loop {
            match self
                .state
                .compare_exchange(UNLOCKED, READ_LOCK, Acquire, Relaxed)
            {
                Ok(_) => return RLockGuard { lock: &self },
                Err(state) => {
                    if state == READ_LOCK {
                        fence(Acquire);
                        return RLockGuard { lock: &self };
                    } else {
                        wait(&self.state, WRITE_LOCK);
                    }
                }
            }
        }
    }

    pub fn write(&self) -> WLockGuard<T> {
        loop {
            match self
                .state
                .compare_exchange(UNLOCKED, WRITE_LOCK, Acquire, Relaxed)
            {
                Ok(_) => return WLockGuard { lock: &self },
                Err(state) => wait(&self.state, state),
            }
        }
    }

    #[inline]
    fn read_unlock(&self) {
        if self.readers.fetch_sub(1, Release) > 1 {
            return;
        }
        // set to a dummy value temporarily, so that no new reader is added
        // while we unlock.
        match self.readers.compare_exchange(0, u32::MAX, Relaxed, Relaxed) {
            Ok(_) => {
                self.state.store(UNLOCKED, Release);
                self.readers.store(0, Relaxed);

                // Wake a writer, if there's any waiting.
                wake_one(&self.state);
            }
            Err(_) => {
                // A new reader has been added. Don't unlock.
            }
        }
    }

    #[inline]
    fn write_unlock(&self) {
        self.state.store(UNLOCKED, Release);
        if self.readers.load(Relaxed) > 0 {
            wake_all(&self.state);
        }
    }
}

pub struct RLockGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<'a, T> Deref for RLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> Drop for RLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.read_unlock();
    }
}

pub struct WLockGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<'a, T> Deref for WLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> DerefMut for WLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for WLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.write_unlock();
    }
}

#[cfg(test)]
mod test {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    use super::RwLock;

    #[test]
    fn multiple_readers() {
        let lock = RwLock::new(0);
        thread::scope(|s| {
            // None of these two threads should need to wait for the other.
            let t1 = s.spawn(|| {
                let start = Instant::now();
                let _guard = lock.read();
                thread::sleep(Duration::from_millis(500));
                start.elapsed()
            });
            let t2 = s.spawn(|| {
                let start = Instant::now();
                let _guard = lock.read();
                thread::sleep(Duration::from_millis(500));
                start.elapsed()
            });

            let t1 = t1.join().unwrap();
            let t2 = t2.join().unwrap();
            assert!(t1 < Duration::from_millis(600));
            assert!(t2 < Duration::from_millis(600));
        });
    }

    #[test]
    fn readers_wait_for_writer() {
        let lock = RwLock::new(0);
        thread::scope(|s| {
            let writer = lock.write();

            let t1 = s.spawn(|| {
                let start = Instant::now();
                let _guard = lock.read();
                start.elapsed()
            });
            let t2 = s.spawn(|| {
                let start = Instant::now();
                let _guard = lock.read();
                start.elapsed()
            });

            thread::sleep(Duration::from_millis(500));
            drop(writer);

            let t1 = t1.join().unwrap();
            let t2 = t2.join().unwrap();
            assert!(t1 > Duration::from_millis(500));
            assert!(t2 > Duration::from_millis(500));
        });
    }

    #[test]
    fn writer_waits_for_readers() {
        let lock = RwLock::new(0);
        thread::scope(|s| {
            let reader = lock.read();

            let reader_2 = s.spawn(|| {
                let start = Instant::now();
                let _guard = lock.read();
                start.elapsed()
            });
            let writer = s.spawn(|| {
                let start = Instant::now();
                let _guard = lock.write();
                start.elapsed()
            });

            thread::sleep(Duration::from_millis(500));
            drop(reader);

            let reader_2 = reader_2.join().unwrap();
            let writer = writer.join().unwrap();
            assert!(reader_2 < Duration::from_millis(10));
            assert!(writer > Duration::from_millis(500));
        });
    }
}
