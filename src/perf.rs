//! Lightweight process metrics, per-feature memory measurement and the
//! automatic memory optimizer.
//!
//! Windows: private working set (PROCESS_MEMORY_COUNTERS_EX2), private bytes
//! (commit), working set and CPU time via GetProcessTimes.
//! Other OSes (development only): /proc/self.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default)]
pub struct MemSample {
    /// Private working set (Windows) / anonymous RSS (Linux), bytes.
    pub private_ws: u64,
    /// Private bytes / commit charge (Windows) / VmData-ish (Linux), bytes.
    pub private_bytes: u64,
    /// Total working set (resident set), bytes.
    pub working_set: u64,
    /// Total CPU time used by the process so far.
    pub cpu_time: Duration,
}

#[cfg(windows)]
pub fn sample() -> MemSample {
    use windows::Win32::System::ProcessStatus::*;
    use windows::Win32::System::Threading::*;
    use windows::Win32::Foundation::FILETIME;
    let mut s = MemSample::default();
    unsafe {
        let h = GetCurrentProcess();
        let mut c2 = PROCESS_MEMORY_COUNTERS_EX2::default();
        c2.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
        if GetProcessMemoryInfo(h, &mut c2 as *mut _ as *mut PROCESS_MEMORY_COUNTERS, c2.cb).is_ok() {
            s.private_ws = c2.PrivateWorkingSetSize as u64;
            s.private_bytes = c2.PrivateUsage as u64;
            s.working_set = c2.WorkingSetSize as u64;
        } else {
            let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
            c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
            if GetProcessMemoryInfo(h, &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, c.cb).is_ok() {
                s.private_bytes = c.PrivateUsage as u64;
                s.working_set = c.WorkingSetSize as u64;
                s.private_ws = c.WorkingSetSize as u64;
            }
        }
        let (mut a, mut b, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        if GetProcessTimes(h, &mut a, &mut b, &mut k, &mut u).is_ok() {
            let f = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
            s.cpu_time = Duration::from_nanos((f(k) + f(u)) * 100);
        }
    }
    s
}

#[cfg(not(windows))]
pub fn sample() -> MemSample {
    let mut s = MemSample::default();
    if let Ok(st) = std::fs::read_to_string("/proc/self/status") {
        for line in st.lines() {
            let kb = |l: &str| l.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) * 1024;
            if line.starts_with("RssAnon:") {
                s.private_ws = kb(line);
            } else if line.starts_with("VmRSS:") {
                s.working_set = kb(line);
            } else if line.starts_with("VmData:") {
                s.private_bytes = kb(line);
            }
        }
    }
    if let Ok(st) = std::fs::read_to_string("/proc/self/stat") {
        // fields after the ")" : utime is 14th, stime 15th overall
        if let Some(rest) = st.rsplit(')').next() {
            let f: Vec<&str> = rest.split_whitespace().collect();
            let ticks = f.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0)
                + f.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            s.cpu_time = Duration::from_millis(ticks * 10);
        }
    }
    s
}

/// Ask Windows to trim the working set (pages move to standby; nothing is lost).
pub fn trim_working_set() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
        use windows::Win32::System::Threading::GetCurrentProcess;
        let _ = EmptyWorkingSet(GetCurrentProcess());
    }
}

pub fn cpu_count() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

/// Rolling monitor. Sampling is cheap (one syscall) and only done every
/// `interval` (1 s while the Performance page is open, 5 s otherwise).
pub struct Monitor {
    pub last: MemSample,
    pub peak_private_ws: u64,
    pub cpu_percent: f32,
    pub history: VecDeque<f32>,
    last_at: Instant,
    last_cpu: Duration,
    ncpu: usize,
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

impl Monitor {
    pub fn new() -> Self {
        let s = sample();
        Monitor {
            last: s,
            peak_private_ws: s.private_ws,
            cpu_percent: 0.0,
            history: VecDeque::with_capacity(120),
            last_at: Instant::now(),
            last_cpu: s.cpu_time,
            ncpu: cpu_count(),
        }
    }

    /// Returns true if a new sample was taken.
    pub fn tick(&mut self, interval: Duration) -> bool {
        let now = Instant::now();
        let dt = now - self.last_at;
        if dt < interval {
            return false;
        }
        let s = sample();
        let dcpu = s.cpu_time.saturating_sub(self.last_cpu);
        self.cpu_percent = (dcpu.as_secs_f32() / dt.as_secs_f32() / self.ncpu as f32 * 100.0).clamp(0.0, 100.0);
        self.last_cpu = s.cpu_time;
        self.last_at = now;
        self.last = s;
        self.peak_private_ws = self.peak_private_ws.max(s.private_ws);
        if self.history.len() >= 120 {
            self.history.pop_front();
        }
        self.history.push_back(mb(s.private_ws));
        true
    }

    pub fn current_mb(&self) -> f32 {
        mb(self.last.private_ws)
    }
}

pub fn mb(b: u64) -> f32 {
    b as f32 / (1024.0 * 1024.0)
}

// ---------------------------------------------------------------------------
// Per-feature measurement

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Measurement {
    /// Mean measured increase in private working set (MB) after enabling.
    pub delta_mb: f32,
    pub samples: u32,
    pub measured_at: i64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Measurements {
    pub by_feature: BTreeMap<String, Measurement>,
}

impl Measurements {
    fn path() -> std::path::PathBuf {
        crate::util::data_dir().join("measurements.toml")
    }
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = crate::util::atomic_write(&Self::path(), s.as_bytes());
        }
    }
    pub fn record(&mut self, id: &str, delta_mb: f32) {
        let e = self.by_feature.entry(id.to_string()).or_insert(Measurement { delta_mb: 0.0, samples: 0, measured_at: 0 });
        // running mean
        e.delta_mb = (e.delta_mb * e.samples as f32 + delta_mb.max(0.0)) / (e.samples + 1) as f32;
        e.samples += 1;
        e.measured_at = crate::util::now_unix();
    }
}

/// Pending measurement: baseline taken when a feature is enabled; read again
/// after it has had time to allocate (artwork loads, windows open...).
pub struct PendingMeasure {
    pub id: String,
    pub baseline: u64,
    pub at: Instant,
}

pub const MEASURE_SETTLE: Duration = Duration::from_secs(6);

/// Only accept a measurement if no other toggle happened in the window and the
/// reading is plausible; negative values are clamped to 0 (allocator noise).
pub fn finish_measure(p: &PendingMeasure, now: &MemSample) -> Option<f32> {
    let d = now.private_ws as i64 - p.baseline as i64;
    let d = d as f32 / (1024.0 * 1024.0);
    if d.abs() > 1024.0 {
        return None;
    }
    Some(d.max(0.0))
}

// ---------------------------------------------------------------------------
// Memory optimizer

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Pressure {
    #[default]
    Normal,
    /// > 85 % of target
    Elevated,
    /// > 100 % of target
    Over,
}

/// Hysteresis: pressure rises immediately, falls only when 10 % below the threshold.
pub fn pressure(current_mb: f32, target_mb: f32, prev: Pressure) -> Pressure {
    let r = current_mb / target_mb.max(1.0);
    let raw = if r > 1.0 {
        Pressure::Over
    } else if r > 0.85 {
        Pressure::Elevated
    } else {
        Pressure::Normal
    };
    if raw >= prev {
        return raw;
    }
    // Falling: each level is only left once usage is clearly below it.
    let mut level = prev;
    if level == Pressure::Over && r < 0.90 {
        level = Pressure::Elevated;
    }
    if level == Pressure::Elevated && r < 0.75 {
        level = Pressure::Normal;
    }
    level
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sample_nonzero() {
        let s = sample();
        assert!(s.working_set > 0);
    }
    #[test]
    fn hysteresis() {
        let mut p = Pressure::Normal;
        p = pressure(180.0, 200.0, p);
        assert_eq!(p, Pressure::Elevated);
        p = pressure(210.0, 200.0, p);
        assert_eq!(p, Pressure::Over);
        p = pressure(195.0, 200.0, p); // not yet below 90 %
        assert_eq!(p, Pressure::Over);
        p = pressure(170.0, 200.0, p);
        assert_eq!(p, Pressure::Elevated);
        p = pressure(160.0, 200.0, p);
        assert_eq!(p, Pressure::Elevated);
        p = pressure(140.0, 200.0, p);
        assert_eq!(p, Pressure::Normal);
    }
    #[test]
    fn measurement_running_mean() {
        let mut m = Measurements::default();
        m.record("x", 4.0);
        m.record("x", 8.0);
        m.record("x", -3.0);
        let e = &m.by_feature["x"];
        assert_eq!(e.samples, 3);
        assert!((e.delta_mb - 4.0).abs() < 1e-4);
    }
}
