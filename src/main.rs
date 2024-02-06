use atomics::{once_data, processor, spinlock};

fn main() {
    once_data::check::run();
    spinlock::check::run();
    processor::run();
}
