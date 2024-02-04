use std::{
    cell::UnsafeCell,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{fence, AtomicUsize, Ordering::*},
};

struct Inner<T> {
    /// Total number of `Arc` instances.
    data_ref_count: AtomicUsize,
    /// Total number of `Arc` and `Weak` instances.
    alloc_ref_count: AtomicUsize,
    /// Data will be dropped if only weak pointers are left.
    data: UnsafeCell<Option<T>>,
}

/// Arc (Atomically Reference Counted) is a thread-safe version of `Rc`.
///
/// `Arc<T>` provides a shared ownership of `T`, allocating it on heap.
pub struct Arc<T> {
    weak: Weak<T>,
}

impl<T> Arc<T> {
    pub fn new(data: T) -> Self {
        let inner = Box::leak(Box::new(Inner {
            data_ref_count: AtomicUsize::new(1),
            alloc_ref_count: AtomicUsize::new(1),
            data: UnsafeCell::new(Some(data)),
        }));
        let weak = Weak {
            inner: NonNull::from(inner),
        };
        Self { weak }
    }

    fn inner(&self) -> &Inner<T> {
        self.weak.inner()
    }

    pub fn strong_count(&self) -> usize {
        self.inner().data_ref_count.load(Relaxed)
    }

    /// Get a mutable reference to the underlying data, only if this is the only
    /// reference to it.
    // The function doesn't take `&mut self` as an argument so that it can only
    // be called as `Arc::get_mut(&mut a)`. This is advisable for types that
    // implement `Deref`, to avoid ambiguity with a similarly named method on
    // the underlying `T`.
    pub fn get_mut(arc: &mut Self) -> Option<&mut T> {
        if arc.inner().alloc_ref_count.load(Relaxed) == 1 {
            fence(Acquire);
            // Safety: there's only one Arc, to which we have an exclusive
            // access.
            let inner = unsafe { arc.weak.inner.as_mut() };
            let option = inner.data.get_mut();
            let data = option.as_mut().unwrap();
            Some(data)
        } else {
            None
        }
    }

    pub fn downgrade(arc: &Self) -> Weak<T> {
        arc.weak.clone()
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let data = self.inner().data.get();
        // Safety: Since this Arc exists, the data exists.
        unsafe { (*data).as_ref().unwrap() }
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        if self.inner().data_ref_count.fetch_add(1, Relaxed) > usize::MAX / 2 {
            // too many references
            std::process::abort();
        }
        let weak = self.weak.clone();
        Self { weak }
    }
}

impl<T> Drop for Arc<T> {
    fn drop(&mut self) {
        // This needs to be synchronized only when the Inner struct is getting
        // dropped.
        let prev_count = self.inner().data_ref_count.fetch_sub(1, Release);
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

            let data = self.inner().data.get();
            // Safety: The data reference count is zero. So there isn't any
            // remaining Arc to access this.
            unsafe { *data = None };
        }
    }
}

pub struct Weak<T> {
    inner: NonNull<Inner<T>>,
}

/// `Arc<T>` can be passed between threads, if `T` can be. Since `Arc<T>` also
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
            return Some(Arc { weak: self.clone() });
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
