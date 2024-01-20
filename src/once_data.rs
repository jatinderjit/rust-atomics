use std::ops::Deref;
use std::sync::atomic::{AtomicPtr, Ordering};

pub struct OnceData<F, T>
where
    F: Fn() -> T,
{
    f: F,
    data: AtomicPtr<T>,
}

impl<F, T> OnceData<F, T>
where
    F: Fn() -> T,
{
    // `f` is the function that returns data. It's possible that `f` is will be
    // called multiple times. But `get` is guaranteed to always return the same
    // value.
    pub fn new(f: F) -> Self {
        Self {
            f,
            data: AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    fn get(&self) -> &'static T {
        let mut p = self.data.load(Ordering::Acquire);
        if p.is_null() {
            let data = Box::into_raw(Box::new((self.f)()));
            p = match self
                .data
                .compare_exchange_weak(p, data, Ordering::Release, Ordering::Acquire)
            {
                Ok(_) => {
                    self.data.store(data, Ordering::Release);
                    data
                }
                Err(d) => {
                    drop(unsafe { Box::from_raw(data) });
                    d
                }
            }
        }
        unsafe { &*p }
    }
}

impl<F, T: 'static> Deref for OnceData<F, T>
where
    F: Fn() -> T,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

pub mod check {
    use super::OnceData;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct Data {
        value: u32,
    }

    fn get_data() -> Data {
        static VALUE: AtomicU32 = AtomicU32::new(0);
        Data {
            value: VALUE.fetch_add(1, Ordering::Relaxed),
        }
    }

    pub fn run() {
        let once_data = OnceData::new(get_data);
        println!("start: {}", get_data().value);
        std::thread::scope(|s| {
            for _ in 0..100 {
                s.spawn(|| {
                    let _d = &once_data.value;
                    println!("{}", _d);
                });
            }
        });
        println!("final: {}", get_data().value);
    }
}
