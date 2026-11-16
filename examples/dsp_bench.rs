//! CPU / memory benchmark for the real-time EQ (`audio::dsp::Processor`).
//!
//! Feeds 60 s of 48 kHz stereo noise through the processor in 480-frame
//! callbacks (10 ms — a typical WASAPI shared-mode period) and reports the time
//! per callback, the share of one CPU core needed for real-time playback, and
//! heap allocations made inside the processing loop (counted by a wrapping
//! global allocator). Run: `cargo run --release --example dsp_bench`

use aqua_glass::audio::dsp::{DspParams, Processor, SharedDsp};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static A: Counting = Counting;

const RATE: u32 = 48_000;
const CB: usize = 480;
const SECONDS: usize = 60;

fn noise(n: usize) -> Vec<f32> {
    let mut x: u32 = 0x1234_5678;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32 - 0.5) * 0.5
        })
        .collect()
}

fn run(name: &str, setup: &SharedDsp, changing: bool) {
    let src = noise(RATE as usize * 2); // 1 s, looped
    let mut buf = vec![0f32; CB * 2];
    let mut proc = Processor::new(RATE);
    let callbacks = SECONDS * RATE as usize / CB;
    let mut times = Vec::with_capacity(callbacks);
    // warm-up (first parameter pickup)
    buf.copy_from_slice(&src[..CB * 2]);
    proc.process(setup, &mut buf);
    let base = ALLOCS.load(Ordering::Relaxed);
    let mut p = DspParams { eq_on: true, eq_gains_db: [4.0, 3.0, 1.0, -1.0, -1.5, 0.0, 1.5, 3.0, 3.5, 4.0], bass_on: true, bass_db: 6.0, preamp_db: -3.0, balance: 0.1 };
    let mut ui_allocs = 0;
    let start = Instant::now();
    for i in 0..callbacks {
        let off = (i * CB * 2) % (src.len() - CB * 2);
        buf.copy_from_slice(&src[off..off + CB * 2]);
        if changing && i % 2 == 0 {
            // a slider being dragged: new values every 20 ms (faster than the UI can send them)
            let before = ALLOCS.load(Ordering::Relaxed);
            p.eq_gains_db[i % 10] = ((i % 48) as f32 / 2.0) - 12.0;
            p.bass_db = (i % 30) as f32 / 2.0;
            setup.set(p.clone());
            ui_allocs += ALLOCS.load(Ordering::Relaxed) - before;
        }
        let t = Instant::now();
        proc.process(setup, &mut buf);
        times.push(t.elapsed().as_nanos() as f64);
        std::hint::black_box(&buf);
    }
    let total = start.elapsed().as_secs_f64();
    let rt_allocs = ALLOCS.load(Ordering::Relaxed) - base - ui_allocs;
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let p99 = times[times.len() * 99 / 100];
    let max = *times.last().unwrap();
    let budget = CB as f64 / RATE as f64 * 1e9;
    println!(
        "{name:<34} mean {:>7.2} µs  p99 {:>7.2} µs  max {:>7.2} µs per 10 ms callback | {:>6.3} % of one core | {:>5.1} ns/frame | allocs in audio path: {rt_allocs} | wall {total:.2}s",
        mean / 1e3,
        p99 / 1e3,
        max / 1e3,
        mean / budget * 100.0,
        mean / CB as f64,
    );
}

fn main() {
    // `dsp_bench response <10 eq gains csv> <bass dB> <rate> <freq>...`: the curve the
    // processor applies (used by bench/eq_bench.py to check the recorded output)
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.first().map(|s| s.as_str()) == Some("response") {
        let mut g = [0f32; 10];
        for (i, v) in a[1].split(',').take(10).enumerate() {
            g[i] = v.parse().unwrap_or(0.0);
        }
        let bass: f32 = a[2].parse().unwrap_or(0.0);
        let p = DspParams { eq_on: true, eq_gains_db: g, bass_on: bass != 0.0, bass_db: bass, ..Default::default() };
        let rate: f32 = a[3].parse().unwrap_or(48_000.0);
        for f in &a[4..] {
            println!("{f} {:.3}", aqua_glass::audio::dsp::response_db_at(&p, f.parse().unwrap_or(1000.0), rate));
        }
        return;
    }
    println!("Processor state: {} bytes; shared parameters: {} bytes", std::mem::size_of::<Processor>(), std::mem::size_of::<SharedDsp>());
    let off = SharedDsp::new();
    run("EQ off (bypass)", &off, false);
    let pre = SharedDsp::new();
    pre.set(DspParams { preamp_db: -3.0, ..Default::default() });
    run("preamp only", &pre, false);
    let full = SharedDsp::new();
    full.set(DspParams { eq_on: true, eq_gains_db: [4.0, 3.0, 1.0, -1.0, -1.5, 0.5, 1.5, 3.0, 3.5, 4.0], bass_on: true, bass_db: 6.0, preamp_db: -3.0, balance: 0.1 });
    run("10 bands + bass + preamp + balance", &full, false);
    let moving = SharedDsp::new();
    run("…while sliders move every 20 ms", &moving, true);
}
