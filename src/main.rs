use std::{thread, time::Instant};

use atomics::{mutex::Mutex, once_data, processor, spinlock};

fn main() {
    if false {
        once_data::check::run();
        spinlock::check::run();
        processor::run();
    }
    bench_mutex();
}

fn bench_mutex() {
    let m = Mutex::new(0);
    std::hint::black_box(&m);
    let start = Instant::now();
    thread::scope(|s| {
        for _ in 0..10 {
            s.spawn(|| {
                for _ in 0..1_000_000 {
                    *m.lock() += 1;
                }
            });
        }
    });
    let duration = start.elapsed();
    println!("locked {} times in {:?}", *m.lock(), duration);
}
