# Arc

From the source code of [Arc](https://github.com/rust-lang/rust/blob/master/library/alloc/src/sync.rs#L2387-L2394):

As explained in the [Boost documentation](www.boost.org/doc/libs/1_55_0/doc/html/atomic/usage_examples.html),

> It is important to enforce any possible access to the object in one
> thread (through an existing reference) to _happen before_ deleting
> the object in a different thread. This is achieved by a "release"
> operation after dropping a reference (any access to the object
> through this reference must obviously happened before), and an
> "acquire" operation before deleting the object.
> It is important to enforce any possible access to the object in one
> thread (through an existing reference) to _happen before_ deleting
> the object in a different thread. This is achieved by a "release"
> operation after dropping a reference (any access to the object
> through this reference must obviously happened before), and an
> "acquire" operation before deleting the object.
