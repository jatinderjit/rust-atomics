use std::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T> Sync for SpinLock<T> where T: Send {}

impl<T> SpinLock<T> {
    pub fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        while self.locked.swap(true, Ordering::Acquire) {
            std::hint::spin_loop();
        }
        SpinLockGuard { lock: &self }
    }

    fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<'a, T> Deref for SpinLockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // Safety: The existence of this Guard instance guarantees this is the
        // exclusive reference to the data.
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // Safety: The existence of this Guard instance guarantees this is the
        // exclusive reference to the data.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.unlock();
    }
}

pub mod check {
    use super::SpinLock;
    use std::{thread, time::Duration};

    pub fn run() {
        run1();
        run2();
    }

    fn run1() {
        let lock = SpinLock::new(-1);
        thread::scope(|s| {
            for i in 0..2 {
                let lock = &lock;
                s.spawn(move || {
                    println!("Locking from thread {i}");
                    let mut guard = lock.lock();
                    println!("Locked from thread {i}");
                    println!("value from thread {i}: {}", *guard);
                    *guard = i;
                    println!("Set value: {i}");
                    thread::sleep(Duration::from_secs(2));
                    println!("Unlocking from thread {i}");
                    drop(guard);
                    println!("Unlocked from thread {i}");
                });
            }
        });
    }

    fn run2() {
        let lock = SpinLock::new(0 as usize);
        const THREADS: usize = 1000;
        thread::scope(|s| {
            for _ in 0..THREADS {
                s.spawn(|| {
                    let mut guard = lock.lock();
                    *guard += 1;
                });
            }
        });
        assert_eq!(*lock.lock(), THREADS);
    }
}
