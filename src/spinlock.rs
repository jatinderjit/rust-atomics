use std::sync::atomic::{AtomicBool, Ordering};

pub struct SpinLock {
    locked: AtomicBool,
}

impl SpinLock {
    pub fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
        }
    }

    pub fn lock(&self) {
        while self.locked.swap(true, Ordering::Acquire) {
            std::hint::spin_loop();
        }
    }

    pub fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

pub mod check {
    use super::SpinLock;
    use std::{thread, time::Duration};

    pub fn run() {
        let lock = SpinLock::new();
        thread::scope(|s| {
            for i in 0..2 {
                let lock = &lock;
                s.spawn(move || {
                    println!("Locking from thread {i}");
                    lock.lock();
                    println!("Locked from thread {i}");
                    thread::sleep(Duration::from_secs(2));
                    println!("Unlocking from thread {i}");
                    lock.unlock();
                    println!("Unlocked from thread {i}");
                });
            }
        });
    }
}
