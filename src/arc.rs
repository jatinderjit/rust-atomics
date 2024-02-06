use std::{
    cell::UnsafeCell,
    mem::ManuallyDrop,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{fence, AtomicUsize, Ordering::*},
};

struct Inner<T> {
    /// Total number of `Arc` instances.
    data_ref_count: AtomicUsize,
    /// Total number of `Weak` instances + 1.
    /// The extra one represents the presence of an `Arc` instance. When all the
    /// `Arc` instances are dropped, this value will be decremented once.
    ///
    /// Inner will be dropped once this count goes to zero.
    alloc_ref_count: AtomicUsize,
    /// Data will be dropped if only weak pointers are left.
    data: UnsafeCell<ManuallyDrop<T>>,
}

/// Arc (Atomically Reference Counted) is a thread-safe version of `Rc`.
///
/// `Arc<T>` provides a shared ownership of `T`, allocating it on heap.
pub struct Arc<T> {
    inner: NonNull<Inner<T>>,
}
/// `Arc<T>` can be passed between threads, if `T` can be. Since `Arc<T>` also
/// provides a shared reference, sending it across threads might result in
/// shared references which are not synchronized by default. So `Send` should
/// also implement `Sync` for safe reference from multiple threads.
unsafe impl<T: Send + Sync> Send for Arc<T> {}
unsafe impl<T: Send + Sync> Sync for Arc<T> {}

impl<T> Arc<T> {
    pub fn new(data: T) -> Self {
        Self {
            inner: NonNull::from(Box::leak(Box::new(Inner {
                data_ref_count: AtomicUsize::new(1),
                alloc_ref_count: AtomicUsize::new(1),
                data: UnsafeCell::new(ManuallyDrop::new(data)),
            }))),
        }
    }

    fn inner(&self) -> &Inner<T> {
        unsafe { self.inner.as_ref() }
    }

    pub fn strong_count(&self) -> usize {
        self.inner().data_ref_count.load(Relaxed)
    }

    /// Get a mutable reference to the underlying data, only if this is the only
    /// reference to it.
    ///
    // The function doesn't take `&mut self` as an argument so that it can only
    // be called as `Arc::get_mut(&mut a)`. This is advisable for types that
    // implement `Deref`, to avoid ambiguity with a similarly named method on
    // the underlying `T`.
    pub fn get_mut(arc: &mut Self) -> Option<&mut T> {
        let inner = arc.inner();

        // Acquire to synchronize with the `Arc::drop`'s `Release`, to ensure
        // that every access former `Arc` clones has happened before this new
        // exclusive access.
        //
        // Set the `alloc_ref_count` to a value that will prevent the creation
        // of `Weak` clones while we run these checks.
        if inner
            .alloc_ref_count
            .compare_exchange(1, usize::MAX, Acquire, Relaxed)
            .is_err()
        {
            return None;
        }

        let is_unique = inner.data_ref_count.load(Relaxed) == 1;
        inner.alloc_ref_count.store(1, Release);
        if !is_unique {
            return None;
        }

        // Acquire to match `Arc::drop`'s decrement, to ensure nothing else is
        // accessing the data.
        fence(Acquire);

        // Safety: there's only one Arc, to which we have an exclusive access.
        Some(unsafe { &mut *arc.inner().data.get() })
    }

    pub fn downgrade(arc: &Self) -> Weak<T> {
        let inner = arc.inner();
        let mut count = inner.alloc_ref_count.load(Relaxed);
        loop {
            if count == usize::MAX {
                std::hint::spin_loop();
                count = inner.alloc_ref_count.load(Relaxed);
                continue;
            }

            assert!(count < usize::MAX - 1);

            // Acquire synchronizes with `get_mut`'s `Release` store.
            if let Err(e) =
                inner
                    .alloc_ref_count
                    .compare_exchange_weak(count, count + 1, Acquire, Relaxed)
            {
                count = e;
                continue;
            }
            return Weak { inner: arc.inner };
        }
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let data = self.inner().data.get();
        // Safety: Since this Arc exists, the data exists.
        unsafe { (*data).deref() }
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        if self.inner().data_ref_count.fetch_add(1, Relaxed) > usize::MAX / 2 {
            // too many references
            std::process::abort();
        }
        Arc { inner: self.inner }
    }
}

impl<T> Drop for Arc<T> {
    fn drop(&mut self) {
        // This needs to be synchronized only when the Inner struct is getting
        // dropped.
        let prev_count = self.inner().data_ref_count.fetch_sub(1, Release);
        if prev_count != 1 {
            return;
        }
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

        // Safety: There are no more `Arc` instances. This field won't be
        // accessed anymore.
        unsafe {
            ManuallyDrop::drop(&mut *self.inner().data.get());
        }

        // Since there's no `Arc` instance left, decrement the counter
        // representing the presence of an `Arc` instance.
        // If there isn't any other `Weak` instance, `Inner` will be dropped.
        drop(Weak { inner: self.inner });
    }
}

pub struct Weak<T> {
    inner: NonNull<Inner<T>>,
}

/// `Weak<T>` can be passed between threads, if `T` can be. Since `Weak<T>` also
/// provides a shared reference, sending it across threads might result in
/// shared references which are not synchronized by default. So `Send` should
/// also implement `Sync` for safe reference from multiple threads.
unsafe impl<T: Send + Sync> Send for Weak<T> {}
unsafe impl<T: Send + Sync> Sync for Weak<T> {}

impl<T> Weak<T> {
    fn inner(&self) -> &Inner<T> {
        unsafe { self.inner.as_ref() }
    }

    pub fn upgrade(&self) -> Option<Arc<T>> {
        let mut count = self.inner().data_ref_count.load(Relaxed);
        loop {
            if count == 0 {
                return None;
            }
            assert!(count <= usize::MAX / 2);
            if let Err(e) =
                self.inner()
                    .data_ref_count
                    .compare_exchange(count, count + 1, Relaxed, Relaxed)
            {
                count = e;
                continue;
            }
            return Some(Arc { inner: self.inner });
        }
    }
}

impl<T> Clone for Weak<T> {
    fn clone(&self) -> Self {
        if self.inner().alloc_ref_count.fetch_add(1, Relaxed) > usize::MAX / 2 {
            // Too many references!
            std::process::abort();
        };
        Self { inner: self.inner }
    }
}

impl<T> Drop for Weak<T> {
    fn drop(&mut self) {
        if self.inner().alloc_ref_count.fetch_sub(1, Relaxed) == 1 {
            fence(Acquire);
            unsafe { drop(Box::from_raw(self.inner.as_ptr())) };
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        sync::{atomic::Ordering::*, Mutex},
        thread,
    };

    use super::{Arc, Weak};

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

    #[test]
    fn downgrade() {
        let arc = Arc::new(1);
        let weak = Arc::downgrade(&arc);
        assert_eq!(weak.inner().data_ref_count.load(Relaxed), 1);
        assert_eq!(weak.inner().alloc_ref_count.load(Relaxed), 2);

        drop(weak);
        assert_eq!(arc.inner().alloc_ref_count.load(Relaxed), 1);
    }

    #[test]
    fn upgrade_fail() {
        let arc = Arc::new(1);
        let weak = Arc::downgrade(&arc);

        drop(arc);
        assert!(Weak::upgrade(&weak).is_none());
    }

    #[test]
    fn upgrade_success() {
        let arc = Arc::new(1);
        let _arc2 = Arc::clone(&arc);
        let weak = Arc::downgrade(&arc);
        drop(arc);

        assert!(Weak::upgrade(&weak).is_some());
    }
}
