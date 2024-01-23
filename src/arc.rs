use std::{
    ops::Deref,
    sync::atomic::{AtomicU32, Ordering::Relaxed},
};

struct Inner<T> {
    value: T,
    count: AtomicU32,
}

pub struct Arc<T> {
    inner: *const Inner<T>,
}

impl<T> Arc<T> {
    pub fn new(value: T) -> Self {
        let inner = Inner {
            value,
            count: AtomicU32::new(1),
        };
        Self {
            inner: Box::into_raw(Box::new(inner)),
        }
    }

    pub fn strong_count(&self) -> u32 {
        unsafe { &*self.inner }.count.load(Relaxed)
    }
}

unsafe impl<T> Send for Arc<T> {}
unsafe impl<T> Sync for Arc<T> where T: Sync {}

impl<T> Deref for Arc<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &(unsafe { &*self.inner }).value
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        unsafe { &*self.inner }.count.fetch_add(1, Relaxed);
        Self { inner: self.inner }
    }
}

impl<T> Drop for Arc<T> {
    fn drop(&mut self) {
        let prev_count = unsafe { &*self.inner }.count.fetch_sub(1, Relaxed);
        if prev_count == 1 {
            unsafe { drop(Box::from_raw(self.inner.cast_mut())) };
        }
    }
}

#[cfg(test)]
mod test {
    use std::{sync::Mutex, thread};

    use super::Arc;

    #[test]
    fn single_thread() {
        let data = Arc::new(123);
        assert_eq!(Arc::strong_count(&data), 1);

        let data2 = Arc::clone(&data);
        assert_eq!(Arc::strong_count(&data), 2);

        drop(data);
        assert_eq!(Arc::strong_count(&data2), 1);

        assert_eq!(*data2, 123);
    }

    #[test]
    fn multiple_threads() {
        let data = Arc::new(123);
        thread::scope(|s| {
            let data2 = Arc::clone(&data);
            s.spawn(move || {
                assert_eq!(*data2, 123);
                assert_eq!(Arc::strong_count(&data2), 2);
            });
            s.spawn(|| assert_eq!(*data, 123));
        });
        assert_eq!(Arc::strong_count(&data), 1);
    }

    #[test]
    fn sync() {
        let data = Arc::new(Mutex::new(1));
        thread::scope(|s| {
            s.spawn(|| {
                let mut guard = data.lock().unwrap();
                *guard = 2;
            });
        });
        assert_eq!(*data.lock().unwrap(), 2);
    }
}
