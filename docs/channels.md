# Channels

Implementations:

- [Runtime safety](../src/channel1.rs) (with panics!)
- [Compile time safety](../src/channel2.rs)

## Generic Channel

Simply use a `Mutex` and a `Condvar` with send and receive. Use `VecDeque` to
store messages.

```rs
pub struct Channel<T> {
    queue: Mutex<VecDeque<T>>,
    ready: Condvar,
}

impl<T> Channel<T> {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
        }
    }

    pub fn send(&self, message: T) {
        self.queue.lock().unwrap().push_back(message);
        self.ready.notify_one();
    }

    pub fn wait(&self) -> T {
        let mut q = self.queue.lock().unwrap();
        loop {
            if let Some(message) = q.pop_front() {
                return message;
            }
            q = self.ready.wait(q).unwrap();
        }
    }
}
```

- No need to use any atomics or unsafe code.
- Didn't have to think about `Sync` and `Send` traits. Compiler implicitly
  understands that guarantees provided by `Mutex` and `Condvar` allow sharing
  between threads.
- The channel is very flexible.

Problems with this approach:

- It allows any number of sending and receiving threads. The implementation
  won't be optimal in many situations.
- Any send or receive operations will briefly block any other send or receive
  operation, even if there are plenty of messages to be received.
- If `VecDeque::push_back` has to grow the capacity, all sending and receiving
  threads will be blocked.
- The queue might grow without bounds. This might be undesirable in certain
  situations.

## One-Shot Channel

- One of the many types of channels, that allows sending exactly one message from
  one thread to another.
- One option is to simply use `Option` instead of `VecDeque`.

### Version 1

- Use an `UnsafeCell` for storage, and `AtomicBool` to indicate if the message is
  ready.
- `Option` can be used to store the message, but that will be a redundant use of
  memory (and computation for each read/write), since we already have an
  `AtomicBool` to indicate the presence of data.
- So use `MaybeUninit`, which is like an unsafe `Option`, but the user has to
  provided the guarantees.

```rs
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
```

#### Problems

(apart from the obvious unsafe interface)

1. Calling `recieve` before the message `is_ready` can cause undefined behavior.
2. `send` can be called twice. This can cause data races, when the second `send`
   overwrites the data while `receive` is reading.
3. Even if `receive` is synchronized, multiple `send` calls from parallel
   threads can also cause data races.
4. Multiple copies of data can be created if `receive` is called twice, even if
   `T` doesn't implement `Copy`.
5. No drop implementation. If data is never `receive`d, it will never be
   dropped and cause memory leaks. If the message is a `Vec`, not only the
   vector, but all its contents will also be leaked.

### Version 2: Fix receive before send

Safety through runtime checks. Fix the first problem: `receive` before `send`.
We will panic if no message is available.

```rs
    /// Panics if no message is available.
    ///
    /// Call this only once, after verifying that the message is `is_ready`.
    pub unsafe fn receive(&self) -> T {
        if !self.ready.load(Ordering::Acquire) {
            panic!("No message");
        }
        (*self.message.get()).assume_init_read()
    }
```

Since we have `Acquire` here, we can relax the ordering in `is_ready`.

```rs
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
```

Due to the **total modification order**, if `is_ready` returns `true`, then
`receive` is guaranteed to read the value of `self.ready` as `true`, and will
never panic if the message is `is_ready`. So the ordering used inside `is_ready`
doesn't matter.

### Version 3: Fix multiple copies (receives)

We can set `self.ready` to false after reading the message. If `receive` is
called again, then it will panic.

The `receive` method no longer needs to be `unsafe`, since there is no undefined
behavior now.

```rs
    /// Panics if no message is available.
    ///
    /// Call this only once, after verifying that the message is `is_ready`.
    pub fn receive(&self) -> T {
        if !self.ready.swap(false, Ordering::Acquire) {
            panic!("No message");
        }
        unsafe { (*self.message.get()).assume_init_read() }
    }
```

### Version 4: Fix multiple sends

We'll need an extra `AtomicBool` field to check if there are parallel calls.

```rs
pub struct OneShotChannel<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    in_use: AtomicBool,  // new field
    ready: AtomicBool,
}
```

If a message has already been sent, we'll panic. Now `send` also doesn't need to
be `unsafe`.

```rs
    /// Panics if called more than once!
    pub fn send(&self, message: T) {
        if self.in_use.swap(true, Ordering::Relaxed) {
            panic!("Can't send more than one message!")
        }
        unsafe {
            (*self.message.get()).write(message);
        }
        self.ready.store(true, Ordering::Release);
    }
```

Relaxed memory ordering will suffice because of the _total modification order_.

### Version 5: Optimize memory usage

Instead of two `AtomicBool`s, we can do with only one `AtomicU8` to manage the
states.

```rs
const EMPTY: u8 = 0;
const WRITING: u8 = 1;
const READY: u8 = 2;
const DONE: u8 = 3;

pub struct OneShotChannel<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    state: AtomicU8,
}
```

And use `compare_exchange` instead of `swap`.

### Version 6: Fix memory leak

`MaybeUninit` will leak the memory, if a never `receive`d. Implement `Drop` if
message has been sent, but not received.

### Version 7: Safety Through Types

We've protected undefined behavior, but at the risk of a panic if the methods
are used incorrectly. Ideally, the compiler should detect and point out the
misuse.

To prevent a function from being called more than once, it can take the argument
`by value`, which will consume the object for non-`Copy` types.

We will need separate non-`Copy` types to `send` and `receive` to make sure each
can only happen once. The new `Sender` and `Receiver` structs will need a
reference to the common channel (which doesn't need to be public anymore). Let's
use `Arc` for that.

```rs
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
        Sender { channel: Arc::clone(&channel) },
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
```

- Only one message can be sent.
- Only one message can be received.
- But the onus on when to call `receive` is still on the user and the method can
  still panic.
- Due to the channel being wrapped in an `Arc`, there's now an allocation.

### Version 8: Avoid Allocation

This will require a trade-off wrt usage. We'll need to keep a reference to the
channel, while the `Sender` and `Receiver` are in scope. Or opposite: the
lifetimes of `Sender` and `Receiver` will be tied to the `Channel`. Also the
`Channel` is now `pub` again!

```rs
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
        (Sender { channel: self }, Receiver { channel: self })
    }
}

pub struct Sender<'a, T> {
    channel: &'a OneShotChannel<T>,
}

pub struct Receiver<'a, T> {
    channel: &'a OneShotChannel<T>,
}
```

`split` takes `&mut self`, which implies that the exclusive access is applicable
until any one of `Sender` and `Receiver` is in scope. When both are dropped,
`split` can be called again to produce a new pair. The channel is also reset for
this new pair, so that the old message might not creep in.

### Version 9: Blocking Receive

Get rid of the panic! The `Receiver` will park the thread, if no message is
available. The `Sender` now needs a reference to the `Receiver`'s thread. We'll
choose an easy way out, and restrict the `Receiver` by not allowing to to be
`Send` anymore. So the thread calling `split` will be the receiving thread.
