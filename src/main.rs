use atomics::{once_data, spinlock};

fn main() {
    if false {
        once_data::check::run();
    }
    spinlock::check::run();
}
