# Channels

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

Problems (apart from the obvious unsafe interface):

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
/// Safety: Call this only once,
///  after verifying that the message is `is_ready`.
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
