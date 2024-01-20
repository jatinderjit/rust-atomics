/// An implementation of `OneShotChannel` that relies on safety through runtime checks.
use core::panic;
use std::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicU8, Ordering::*},
};

const EMPTY: u8 = 0;
const WRITING: u8 = 1;
const READY: u8 = 2;
const DONE: u8 = 3;

/// Send a message once. Receive the message once.
pub struct OneShotChannel<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    state: AtomicU8,
}

unsafe impl<T> Sync for OneShotChannel<T> where T: Send {}

impl<T> OneShotChannel<T> {
    pub fn new() -> Self {
        OneShotChannel {
            message: UnsafeCell::new(MaybeUninit::uninit()),
            state: AtomicU8::new(EMPTY),
        }
    }

    /// Panics if called more than once!
    pub fn send(&self, message: T) {
        if self
            .state
            .compare_exchange(EMPTY, WRITING, Relaxed, Relaxed)
            .is_err()
        {
            panic!("Can't send more than one message!")
        }
        unsafe {
            (*self.message.get()).write(message);
        }
        self.state.store(READY, Release);
    }

    pub fn is_ready(&self) -> bool {
        self.state.load(Relaxed) == READY
    }

    /// Panics if no message is available.
    ///
    /// Call this only once, after verifying that the message is `is_ready`.
    pub fn receive(&self) -> T {
        match self.state.compare_exchange(READY, DONE, Acquire, Relaxed) {
            Ok(_) => unsafe { (*self.message.get()).assume_init_read() },
            Err(EMPTY) | Err(WRITING) => panic!("No message!"),
            Err(DONE) => panic!("Can't read message more than once!"),
            Err(_) => unreachable!(),
        }
    }
}

impl<T> Drop for OneShotChannel<T> {
    fn drop(&mut self) {
        // Atomic access is unnecessary.
        let state = *self.state.get_mut();

        // MaybeUninit should not be drop if it is not initialized.
        // If it has been received, then the value has been moved, and should not be
        // dropped by us.
        if state == EMPTY || state == DONE {
            return;
        }
        // UnsafeCell::get_mut is a compile time guarantee that this is the only
        // reference.
        unsafe { self.message.get_mut().assume_init_drop() }
    }
}

#[cfg(test)]
mod test {
    use super::OneShotChannel;
    use std::{rc::Rc, thread, time::Duration};

    #[test]
    fn single_thread() {
        let channel = OneShotChannel::new();
        assert!(!channel.is_ready());

        channel.send(123);
        assert!(channel.is_ready());
        assert_eq!(channel.receive(), 123);
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
                assert_eq!(channel.receive(), 123);
            });
            s.spawn(move || {
                thread::sleep(Duration::from_millis(100));
                channel.send(123);
                receiver.thread().unpark();
            });
        })
    }

    #[test]
    #[should_panic(expected = "No message!")]
    fn receive_no_message() {
        let channel = OneShotChannel::<i32>::new();
        channel.receive();
    }

    #[test]
    #[should_panic(expected = "Can't read message more than once!")]
    fn multiple_receives() {
        let channel = OneShotChannel::new();
        channel.send(123);
        channel.receive();
        channel.receive();
    }

    #[test]
    #[should_panic(expected = "Can't send more than one message!")]
    fn multiple_sends() {
        let channel = OneShotChannel::new();
        channel.send(123);
        channel.send(123);
    }

    #[test]
    fn drop_no_receive() {
        let value = Rc::new(123);
        let channel = OneShotChannel::new();

        channel.send(Rc::clone(&value));
        assert_eq!(Rc::strong_count(&value), 2);

        drop(channel);
        assert_eq!(Rc::strong_count(&value), 1);
    }

    #[test]
    fn drop_with_receive() {
        let value = Rc::new(123);
        let channel = OneShotChannel::new();

        channel.send(Rc::clone(&value));
        assert_eq!(Rc::strong_count(&value), 2);

        channel.receive();
        assert_eq!(Rc::strong_count(&value), 1);

        drop(channel);
        assert_eq!(Rc::strong_count(&value), 1);
    }
}
