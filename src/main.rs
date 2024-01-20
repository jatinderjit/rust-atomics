use atomics::{once_data, spinlock};

fn main() {
    once_data::check::run();
    spinlock::check::run();
}
