use std::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, Ordering},
};

pub struct OneShotChannel<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    ready: AtomicBool,
}

unsafe impl<T> Sync for OneShotChannel<T> where T: Send {}

impl<T> OneShotChannel<T> {
    pub fn new() -> Self {
        OneShotChannel {
            message: UnsafeCell::new(MaybeUninit::uninit()),
            ready: AtomicBool::new(false),
        }
    }

    /// Safety: Call this only once!
    pub unsafe fn send(&self, message: T) {
        (*self.message.get()).write(message);
        self.ready.store(true, Ordering::Release);
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Safety: Call this only once,
    /// and that too after verifying that the value is `is_ready`.
    pub unsafe fn receive(&self) -> T {
        (*self.message.get()).assume_init_read()
    }
}

pub mod check {
    use super::OneShotChannel;
    use std::{thread, time::Duration};

    pub fn run() {
        single_thread();
        multiple_threads();
    }

    fn single_thread() {
        let channel = OneShotChannel::new();
        assert!(!channel.is_ready());

        unsafe {
            channel.send(123);
        }
        assert!(channel.is_ready());
        let val = unsafe { channel.receive() };
        assert_eq!(val, 123);
    }

    fn multiple_threads() {
        let channel = OneShotChannel::new();
        thread::scope(|s| {
            let channel = &channel;
            let receiver = s.spawn(|| {
                assert!(!channel.is_ready());
                while !channel.is_ready() {
                    thread::park();
                }
                let val = unsafe { channel.receive() };
                assert_eq!(val, 123);
            });
            s.spawn(move || {
                thread::sleep(Duration::from_millis(100));
                unsafe {
                    channel.send(123);
                }
                receiver.thread().unpark();
            });
        })
    }
}
