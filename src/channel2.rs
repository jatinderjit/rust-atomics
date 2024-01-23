use std::{
    cell::UnsafeCell,
    marker::PhantomData,
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, Ordering::*},
    thread::{self, Thread},
};

pub struct OneShotChannel<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    ready: AtomicBool,
}

impl<T> OneShotChannel<T> {
    pub fn new() -> Self {
        Self {
            message: UnsafeCell::new(MaybeUninit::uninit()),
            ready: AtomicBool::new(false),
        }
    }

    pub fn split<'a>(&'a mut self) -> (Sender<'a, T>, Receiver<'a, T>) {
        // Reset, in case this is called again, once the sender and receiver
        // have "expired". This will also drop the existing channel.
        *self = Self::new();
        let sender = Sender {
            channel: self,
            receiving_thread: thread::current(),
        };
        let receiver = Receiver {
            channel: self,
            _no_send: PhantomData,
        };
        (sender, receiver)
    }
}

unsafe impl<T> Sync for OneShotChannel<T> where T: Send {}

impl<T> Drop for OneShotChannel<T> {
    fn drop(&mut self) {
        if *self.ready.get_mut() {
            unsafe { self.message.get_mut().assume_init_drop() };
        }
    }
}

pub struct Sender<'a, T> {
    channel: &'a OneShotChannel<T>,
    receiving_thread: Thread,
}

pub struct Receiver<'a, T> {
    channel: &'a OneShotChannel<T>,
    _no_send: PhantomData<*const ()>,
}

impl<T> Sender<'_, T> {
    pub fn send(self, message: T) {
        unsafe { (*self.channel.message.get()).write(message) };
        self.channel.ready.store(true, Release);
        self.receiving_thread.unpark();
    }
}

impl<T> Receiver<'_, T> {
    pub fn is_ready(&self) -> bool {
        self.channel.ready.load(Relaxed)
    }

    pub fn receive(self) -> T {
        while !self.channel.ready.swap(false, Acquire) {
            thread::park();
        }
        unsafe { (*self.channel.message.get()).assume_init_read() }
    }
}

#[cfg(test)]
mod test {
    use std::{rc::Rc, thread, time::Duration};

    use super::OneShotChannel;

    #[test]
    fn single_thread() {
        let mut channel = OneShotChannel::new();
        let (sender, receiver) = channel.split();
        assert!(!receiver.is_ready());
        sender.send(123);
        assert!(receiver.is_ready());
        assert_eq!(receiver.receive(), 123);
    }

    #[test]
    fn multiple_threads() {
        let mut channel = OneShotChannel::new();
        let (sender, receiver) = channel.split();
        thread::scope(|s| {
            s.spawn(|| {
                thread::sleep(Duration::from_millis(10));
                sender.send(123);
            });
            assert!(!receiver.is_ready());
            assert_eq!(receiver.receive(), 123);
        });
    }

    #[test]
    fn drop_without_receive() {
        let message = Rc::new(123);
        let mut channel = OneShotChannel::new();
        let (sender, _) = channel.split();
        sender.send(Rc::clone(&message));
        assert_eq!(Rc::strong_count(&message), 2);

        drop(channel);

        assert_eq!(Rc::strong_count(&message), 1);
    }

    #[test]
    fn drop_after_receive() {
        let message = Rc::new(123);
        let mut channel = OneShotChannel::new();
        let (sender, receiver) = channel.split();
        sender.send(Rc::clone(&message));
        assert_eq!(Rc::strong_count(&message), 2);

        let received = receiver.receive();
        assert_eq!(&message, &received);
        assert_eq!(*received, 123);
        assert_eq!(Rc::strong_count(&message), 2);

        drop(received);
        assert_eq!(Rc::strong_count(&message), 1);
    }

    #[test]
    fn channel_reuse() {
        let message = Rc::new(123);
        let mut channel = OneShotChannel::new();
        let (sender, _) = channel.split();
        sender.send(Rc::clone(&message));
        assert_eq!(Rc::strong_count(&message), 2);

        // sender and receiver should be dropped now
        let (sender, receiver) = channel.split();
        assert_eq!(Rc::strong_count(&message), 1);

        sender.send(Rc::clone(&message));
        assert_eq!(Rc::strong_count(&message), 2);

        let received = receiver.receive();
        assert_eq!(Rc::strong_count(&message), 2);

        drop(channel);
        assert_eq!(Rc::strong_count(&message), 2);

        drop(received);
        assert_eq!(Rc::strong_count(&message), 1);
    }
}
