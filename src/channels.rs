use core::panic;
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
        self.ready.load(Ordering::Relaxed)
    }

    /// Panics if no message is available.
    ///
    /// Safety: Call this only once,
    ///  after verifying that the message is `is_ready`.
    pub fn receive(&self) -> T {
        if !self.ready.swap(false, Ordering::Acquire) {
            panic!("No message");
        }
        unsafe { (*self.message.get()).assume_init_read() }
    }
}

#[cfg(test)]
mod test {
    use super::OneShotChannel;
    use std::{thread, time::Duration};

    #[test]
    fn single_thread() {
        let channel = OneShotChannel::new();
        assert!(!channel.is_ready());

        unsafe {
            channel.send(123);
        }
        assert!(channel.is_ready());
        let val = channel.receive();
        assert_eq!(val, 123);
    }

    #[test]
    fn multiple_threads() {
        let channel = OneShotChannel::new();
        thread::scope(|s| {
            let channel = &channel;
            let receiver = s.spawn(|| {
                assert!(!channel.is_ready());
                while !channel.is_ready() {
                    thread::park();
                }
                let val = channel.receive();
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

    #[test]
    #[should_panic(expected = "No message")]
    fn receive_no_message() {
        let channel = OneShotChannel::<i32>::new();
        channel.receive();
    }

    #[test]
    #[should_panic]
    fn multiple_receives() {
        let channel = OneShotChannel::new();
        unsafe {
            channel.send(123);
        }
        channel.receive();
        channel.receive();
    }
}
