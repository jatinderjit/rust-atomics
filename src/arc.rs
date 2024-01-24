use std::{
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{fence, AtomicUsize, Ordering::*},
};

struct Inner<T: ?Sized> {
    /// Total number of `Arc` instances.
    count: AtomicUsize,
    data: T,
}

/// Arc (Atomically Reference Counted) is a thread-safe version of `Rc`.
///
/// `Arc<T>` provides a shared ownership of `T`, allocating it on heap.
pub struct Arc<T: ?Sized> {
    inner: NonNull<Inner<T>>,
}

impl<T> Arc<T> {
    pub fn new(data: T) -> Self {
        let inner = Inner {
            data,
            count: AtomicUsize::new(1),
        };
        let inner = Box::into_raw(Box::new(inner));
        Self {
            inner: unsafe { NonNull::new_unchecked(inner) },
        }
    }

    fn inner(&self) -> &Inner<T> {
        unsafe { &self.inner.as_ref() }
    }

    pub fn strong_count(&self) -> usize {
        self.inner().count.load(Relaxed)
    }

    /// Get a mutable reference to the underlying data, only if this is the only
    /// reference to it.
    // The function doesn't take `&mut self` as an argument so that it can only
    // be called as `Arc::get_mut(&mut a)`. This is advisable for types that
    // implement `Deref`, to avoid ambiguity with a similarly named method on
    // the underlying `T`.
    pub fn get_mut(arc: &mut Self) -> Option<&mut T> {
        if arc.inner().count.load(Relaxed) == 1 {
            fence(Acquire);
            unsafe { Some(&mut arc.inner.as_mut().data) }
        } else {
            None
        }
    }
}

/// `Arc<T>` can be passed between threads, if `T` can be. Since `Arc<T>` also
/// provides a shared reference, sending it across threads might result in
/// shared references which are not synchronized by default. So `Send` should
/// also implement `Sync` for safe reference from multiple threads.
unsafe impl<T: Send + Sync> Send for Arc<T> {}

unsafe impl<T: Send + Sync> Sync for Arc<T> {}

impl<T> Deref for Arc<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.inner().data
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        self.inner().count.fetch_add(1, Relaxed);
        Self { inner: self.inner }
    }
}

impl<T: ?Sized> Drop for Arc<T> {
    fn drop(&mut self) {
        // This needs to be synchronized only when the Inner struct is getting
        // dropped.
        let prev_count = unsafe { self.inner.as_mut() }.count.fetch_sub(1, Release);
        if prev_count == 1 {
            // From the official `std::sync::Arc` docs:
            //
            // This fence is needed to prevent reordering of use of data and the
            // deletion of data. This `Acquire` synchronizes with the `Acquire`
            // of the drop.
            //
            // As explained in the [Boost documentation][1],
            //
            // > It is important to enforce any possible access to the object in one
            // > thread (through an existing reference) to *happen before* deleting
            // > the object in a different thread. This is achieved by a "release"
            // > operation after dropping a reference (any access to the object
            // > through this reference must obviously happened before), and an
            // > "acquire" operation before deleting the object.
            //
            // [1]: (www.boost.org/doc/libs/1_55_0/doc/html/atomic/usage_examples.html)
            fence(Acquire);
            unsafe { drop(Box::from_raw(self.inner.as_mut())) };
        }
    }
}

#[cfg(test)]
mod test {
    use std::{sync::Mutex, thread};

    use super::Arc;

    #[test]
    fn single_thread() {
        let arc = Arc::new(123);
        assert_eq!(Arc::strong_count(&arc), 1);

        let arc2 = Arc::clone(&arc);
        assert_eq!(Arc::strong_count(&arc), 2);

        drop(arc);
        assert_eq!(Arc::strong_count(&arc2), 1);

        assert_eq!(*arc2, 123);
    }

    #[test]
    fn multiple_threads() {
        let arc = Arc::new(123);
        thread::scope(|s| {
            let data2 = Arc::clone(&arc);
            s.spawn(move || {
                assert_eq!(*data2, 123);
                assert_eq!(Arc::strong_count(&data2), 2);
            });
            s.spawn(|| assert_eq!(*arc, 123));
        });
        assert_eq!(Arc::strong_count(&arc), 1);
    }

    #[test]
    fn sync() {
        let arc = Arc::new(Mutex::new(1));
        thread::scope(|s| {
            s.spawn(|| {
                let mut guard = arc.lock().unwrap();
                *guard = 2;
            });
        });
        assert_eq!(*arc.lock().unwrap(), 2);
    }

    #[test]
    fn mut_ref() {
        let mut arc = Arc::new(1);
        let arc2 = Arc::clone(&arc);
        assert!(Arc::get_mut(&mut arc).is_none());
        drop(arc2);
        assert!(Arc::get_mut(&mut arc).is_some());
    }
}
