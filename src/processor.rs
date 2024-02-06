use std::{
    hint::black_box,
    sync::atomic::{AtomicU64, Ordering::*},
    thread,
    time::Instant,
};

const LOOPS: u32 = 1_000_000_000;

/// Naive load: the loop will be optimized away!
#[allow(dead_code)]
fn naive_load() {
    static A: AtomicU64 = AtomicU64::new(0);
    let start = Instant::now();
    for _ in 0..LOOPS {
        A.load(Relaxed);
    }
    println!("Naive load: {:?}", start.elapsed());
}

/// Load values, using `black_box` to avoid the loop being optimized away!.
#[allow(dead_code)]
fn just_load() {
    static A: AtomicU64 = AtomicU64::new(0);
    black_box(&A);
    let start = Instant::now();
    for _ in 0..LOOPS {
        A.load(Relaxed);
    }
    println!("Just load: {:?}", start.elapsed());
}

#[allow(dead_code)]
fn parallel_loads() {
    static A: AtomicU64 = AtomicU64::new(0);
    black_box(&A);

    thread::spawn(|| loop {
        A.load(Relaxed);
    });

    let start = Instant::now();
    for _ in 0..LOOPS {
        A.load(Relaxed);
    }
    println!("parallel_loads: {:?}", start.elapsed());
}

#[allow(dead_code)]
fn store_and_load() {
    static A: AtomicU64 = AtomicU64::new(0);
    black_box(&A);

    thread::spawn(|| loop {
        A.store(0, Relaxed);
    });

    let start = Instant::now();
    for _ in 0..LOOPS {
        A.load(Relaxed);
    }
    println!("store_and_load: {:?}", start.elapsed());
}

/// Compare and exchange that always fails.
#[allow(dead_code)]
fn compare_exchange() {
    static A: AtomicU64 = AtomicU64::new(0);
    black_box(&A);

    thread::spawn(|| loop {
        // never succeeds
        let _ = A.compare_exchange(10, 20, Relaxed, Relaxed);
    });

    let start = Instant::now();
    for _ in 0..LOOPS {
        A.load(Relaxed);
    }
    println!("compare_exchange: {:?}", start.elapsed());
}

#[allow(dead_code)]
fn overlapping_cache_line() {
    static B: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    black_box(&B);
    thread::spawn(|| loop {
        B[0].store(0, Relaxed);
        B[2].store(0, Relaxed);
    });
    let start = Instant::now();
    for _ in 0..LOOPS {
        B[1].load(Relaxed);
    }
    println!("overlapping_cache_line: {:?}", start.elapsed());
}

#[repr(align(64))]
struct Aligned(AtomicU64);

#[allow(dead_code)]
fn non_overlapping_cache_line() {
    static C: [Aligned; 3] = [
        Aligned(AtomicU64::new(0)),
        Aligned(AtomicU64::new(0)),
        Aligned(AtomicU64::new(0)),
    ];
    black_box(&C);
    thread::spawn(|| loop {
        C[0].0.store(0, Relaxed);
        C[2].0.store(0, Relaxed);
    });
    let start = Instant::now();
    for _ in 0..LOOPS {
        C[1].0.load(Relaxed);
    }
    println!("overlapping_cache_line: {:?}", start.elapsed());
}

/// # Benchmark
///
/// - `naive_load` : 125 ns
/// - `just_load` : 320 ms
/// - `parallel_loads` : 330 ms
/// - `store_and_load` : 680 ms
/// - `compare_exchange` : 340 ms
/// - `overlapping_cache_line` : 710 ms
/// - `non_overlapping_cache_line` : 400 ms
pub fn run() {
    naive_load();
    just_load();
    store_and_load();
    compare_exchange();
    overlapping_cache_line();
    non_overlapping_cache_line();
}
