use core::panic;
use std::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::{
        atomic::{AtomicBool, Ordering::*},
        Arc,
    },
};

struct OneShotChannel<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    ready: AtomicBool,
}

unsafe impl<T> Sync for OneShotChannel<T> where T: Send {}

impl<T> Drop for OneShotChannel<T> {
    fn drop(&mut self) {
        if *self.ready.get_mut() {
            unsafe { self.message.get_mut().assume_init_drop() };
        }
    }
}

pub struct Sender<T> {
    channel: Arc<OneShotChannel<T>>,
}

pub struct Receiver<T> {
    channel: Arc<OneShotChannel<T>>,
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let channel = OneShotChannel {
        message: UnsafeCell::new(MaybeUninit::uninit()),
        ready: AtomicBool::new(false),
    };
    let channel = Arc::new(channel);
    (
        Sender {
            channel: Arc::clone(&channel),
        },
        Receiver { channel },
    )
}

impl<T> Sender<T> {
    pub fn send(self, message: T) {
        unsafe { (*self.channel.message.get()).write(message) };
        self.channel.ready.store(true, Release);
    }
}

impl<T> Receiver<T> {
    pub fn is_ready(&self) -> bool {
        self.channel.ready.load(Relaxed)
    }

    pub fn receive(self) -> T {
        if !self.channel.ready.swap(false, Acquire) {
            panic!("No message!")
        }
        unsafe { (*self.channel.message.get()).assume_init_read() }
    }
}

#[cfg(test)]
mod test {
    use std::{rc::Rc, thread, time::Duration};

    use super::channel;

    #[test]
    fn single_thread() {
        let (sender, receiver) = channel();
        assert!(!receiver.is_ready());
        sender.send(123);
        assert!(receiver.is_ready());
        assert_eq!(receiver.receive(), 123);
    }

    #[test]
    fn multiple_threads() {
        let (sender, receiver) = channel();
        thread::scope(|s| {
            let recv_thread = s.spawn(|| {
                // Not really a guarantee!
                assert!(!receiver.is_ready());
                while !receiver.is_ready() {
                    thread::park();
                }
                assert_eq!(receiver.receive(), 123);
            });
            s.spawn(move || {
                thread::sleep(Duration::from_millis(10));
                sender.send(123);
                recv_thread.thread().unpark();
            });
        });
    }

    #[test]
    #[should_panic]
    fn receive_before_send() {
        let (_, receiver) = channel::<i32>();
        receiver.receive();
    }

    #[test]
    fn drop_without_receive() {
        let message = Rc::new(123);
        let (sender, receiver) = channel();
        sender.send(Rc::clone(&message));
        assert_eq!(Rc::strong_count(&message), 2);

        drop(receiver);

        assert_eq!(Rc::strong_count(&message), 1);
    }

    #[test]
    fn drop_after_receive() {
        let message = Rc::new(123);
        let (sender, receiver) = channel();
        sender.send(Rc::clone(&message));
        assert_eq!(Rc::strong_count(&message), 2);

        let received = receiver.receive();
        assert_eq!(&message, &received);
        assert_eq!(*received, 123);
        assert_eq!(Rc::strong_count(&message), 2);

        drop(received);
        assert_eq!(Rc::strong_count(&message), 1);
    }
}
