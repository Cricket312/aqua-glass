//! DSP: 10-band EQ, bass boost, preamp, balance and a linear resampler.
//!
//! The EQ/bass/preamp/balance `Processor` runs in the real-time output callback,
//! after the playback buffer, so setting changes are heard immediately instead of
//! after the buffered audio (≈0.5 s) has played. It never blocks there: new
//! parameters are picked up with `try_lock`. Each stage is skipped entirely when
//! its feature is off. Per-track ReplayGain stays on the producer side.

use crate::config::EQ_FREQS;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq)]
pub struct DspParams {
    pub eq_on: bool,
    pub eq_gains_db: [f32; 10],
    pub bass_on: bool,
    pub bass_db: f32,
    pub preamp_db: f32,
    pub balance: f32,
}

impl Default for DspParams {
    fn default() -> Self {
        DspParams { eq_on: false, eq_gains_db: [0.0; 10], bass_on: false, bass_db: 6.0, preamp_db: 0.0, balance: 0.0 }
    }
}

pub struct SharedDsp {
    pub params: Mutex<DspParams>,
    pub version: AtomicU64,
}

impl SharedDsp {
    pub fn new() -> Self {
        SharedDsp { params: Mutex::new(DspParams::default()), version: AtomicU64::new(1) }
    }
    pub fn set(&self, p: DspParams) {
        let mut g = self.params.lock().unwrap();
        if *g != p {
            *g = p;
            self.version.fetch_add(1, Ordering::Release);
        }
    }
}

impl Default for SharedDsp {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: [f32; 2],
    z2: [f32; 2],
}

impl Biquad {
    fn set(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }
    /// RBJ cookbook peaking EQ.
    fn peaking(&mut self, rate: f32, f0: f32, q: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * (f0 / rate).min(0.49);
        let alpha = w0.sin() / (2.0 * q);
        let c = w0.cos();
        self.set(1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a);
    }
    /// RBJ cookbook low shelf (S = 1).
    fn low_shelf(&mut self, rate: f32, f0: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * (f0 / rate).min(0.49);
        let c = w0.cos();
        let alpha = w0.sin() / 2.0 * std::f32::consts::SQRT_2;
        let sa = 2.0 * a.sqrt() * alpha;
        self.set(
            a * ((a + 1.0) - (a - 1.0) * c + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - sa),
            (a + 1.0) + (a - 1.0) * c + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - sa,
        );
    }
    /// |H(e^jw)| at angular frequency `w` (radians/sample).
    fn magnitude(&self, w: f32) -> f32 {
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let num = ((self.b0 + self.b1 * c1 + self.b2 * c2).powi(2) + (self.b1 * s1 + self.b2 * s2).powi(2)).sqrt();
        let den = ((1.0 + self.a1 * c1 + self.a2 * c2).powi(2) + (self.a1 * s1 + self.a2 * s2).powi(2)).sqrt();
        num / den.max(1e-9)
    }
    #[inline]
    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let y = self.b0 * x + self.z1[ch];
        self.z1[ch] = self.b1 * x - self.a1 * y + self.z2[ch];
        self.z2[ch] = self.b2 * x - self.a2 * y;
        y
    }
}

/// Highest gain (dB) the EQ + bass curve reaches anywhere in 20 Hz–20 kHz — the
/// headroom needed so boosts don't clip. Evaluated on the real filter curves
/// (bands overlap and partly cancel, so adding the largest boosts overstates it).
pub fn peak_gain_db(p: &DspParams) -> f32 {
    let rate = 48_000.0;
    let mut filters = Vec::new();
    if p.eq_on {
        for (i, f) in EQ_FREQS.iter().enumerate() {
            if p.eq_gains_db[i].abs() > 0.05 {
                let mut b = Biquad::default();
                b.peaking(rate, *f, 1.41, p.eq_gains_db[i]);
                filters.push(b);
            }
        }
    }
    if p.bass_on && p.bass_db.abs() > 0.05 {
        let mut b = Biquad::default();
        b.low_shelf(rate, 120.0, p.bass_db);
        filters.push(b);
    }
    if filters.is_empty() {
        return 0.0;
    }
    (0..400)
        .map(|i| {
            let f = 20.0 * 1000f32.powf(i as f32 / 399.0); // 20 Hz .. 20 kHz, log-spaced
            let w = 2.0 * std::f32::consts::PI * f / rate;
            20.0 * filters.iter().map(|b| b.magnitude(w)).product::<f32>().log10()
        })
        .fold(f32::MIN, f32::max)
}

/// Per-producer processing state.
pub struct Processor {
    rate: f32,
    version: u64,
    params: DspParams,
    eq: [Biquad; 10],
    eq_active: [bool; 10],
    bass: Biquad,
    /// Extra linear gain from ReplayGain (local files).
    pub track_gain: f32,
}

impl Processor {
    pub fn new(rate: u32) -> Self {
        Processor {
            rate: rate as f32,
            version: 0,
            params: DspParams::default(),
            eq: [Biquad::default(); 10],
            eq_active: [false; 10],
            bass: Biquad::default(),
            track_gain: 1.0,
        }
    }

    fn refresh(&mut self, shared: &SharedDsp) {
        let v = shared.version.load(Ordering::Acquire);
        if v == self.version {
            return;
        }
        // real-time callback: never wait for the UI thread; retry on the next buffer
        let Ok(g) = shared.params.try_lock() else { return };
        let p = g.clone();
        drop(g);
        self.version = v;
        for (i, f) in EQ_FREQS.iter().enumerate() {
            let g = p.eq_gains_db[i];
            self.eq_active[i] = p.eq_on && g.abs() > 0.05;
            if self.eq_active[i] {
                let mut b = self.eq[i];
                b.peaking(self.rate, *f, 1.41, g);
                self.eq[i] = b;
            }
        }
        if p.bass_on {
            self.bass.low_shelf(self.rate, 120.0, p.bass_db);
        }
        self.params = p;
    }

    /// Process interleaved stereo in place.
    pub fn process(&mut self, shared: &SharedDsp, buf: &mut [f32]) {
        self.refresh(shared);
        let p = &self.params;
        let pre = 10f32.powf(p.preamp_db / 20.0) * self.track_gain;
        let (lg, rg) = ((1.0 - p.balance).min(1.0), (1.0 + p.balance).min(1.0));
        let any_eq = self.eq_active.iter().any(|a| *a);
        let simple = !any_eq && !p.bass_on && (pre - 1.0).abs() < 1e-4 && p.balance.abs() < 1e-4;
        if simple {
            return;
        }
        for frame in buf.chunks_exact_mut(2) {
            for ch in 0..2 {
                let mut x = frame[ch] * pre;
                if any_eq {
                    for (i, b) in self.eq.iter_mut().enumerate() {
                        if self.eq_active[i] {
                            x = b.run(ch, x);
                        }
                    }
                }
                if p.bass_on {
                    x = self.bass.run(ch, x);
                }
                frame[ch] = x * if ch == 0 { lg } else { rg };
            }
        }
        // soft clip protection after boosts
        if any_eq || p.bass_on || pre > 1.0 {
            for s in buf.iter_mut() {
                if s.abs() > 0.95 {
                    *s = s.signum() * (0.95 + (s.abs() - 0.95).tanh() * 0.05);
                }
            }
        }
    }
}

/// Streaming linear resampler for interleaved stereo.
pub struct Resampler {
    from: u32,
    to: u32,
    pos: f64,
    last: [f32; 2],
    primed: bool,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Resampler { from, to, pos: 0.0, last: [0.0; 2], primed: false }
    }
    pub fn passthrough(&self) -> bool {
        self.from == self.to
    }
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.passthrough() {
            out.extend_from_slice(input);
            return;
        }
        let step = self.from as f64 / self.to as f64;
        let frames = input.len() / 2;
        if frames == 0 {
            return;
        }
        if !self.primed {
            self.last = [input[0], input[1]];
            self.primed = true;
        }
        // Virtual stream: index 0 = last frame of the previous chunk, 1..=frames = this chunk.
        let last = self.last;
        let vf = |i: usize| -> [f32; 2] {
            if i == 0 {
                last
            } else {
                [input[(i - 1) * 2], input[(i - 1) * 2 + 1]]
            }
        };
        while (self.pos.floor() as usize) < frames {
            let i = self.pos.floor() as usize;
            let t = (self.pos - i as f64) as f32;
            let a = vf(i);
            let b = vf(i + 1);
            out.push(a[0] + (b[0] - a[0]) * t);
            out.push(a[1] + (b[1] - a[1]) * t);
            self.pos += step;
        }
        self.pos -= frames as f64;
        self.last = [input[(frames - 1) * 2], input[(frames - 1) * 2 + 1]];
    }
}

/// Convert any channel layout to interleaved stereo.
pub fn to_stereo(input: &[f32], channels: usize, out: &mut Vec<f32>) {
    match channels {
        1 => {
            for s in input {
                out.push(*s);
                out.push(*s);
            }
        }
        2 => out.extend_from_slice(input),
        n if n > 2 => {
            for f in input.chunks_exact(n) {
                out.push(f[0]);
                out.push(f[1]);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn sine(freq: f32, rate: f32, n: usize) -> Vec<f32> {
        let mut v = Vec::with_capacity(n * 2);
        for i in 0..n {
            let s = (2.0 * std::f32::consts::PI * freq * i as f32 / rate).sin() * 0.25;
            v.push(s);
            v.push(s);
        }
        v
    }

    #[test]
    fn flat_is_bit_exact() {
        let shared = SharedDsp::new();
        let mut p = Processor::new(44100);
        let mut x = sine(440.0, 44100.0, 1000);
        let orig = x.clone();
        p.process(&shared, &mut x);
        assert_eq!(x, orig);
    }

    #[test]
    fn eq_boosts_band() {
        let shared = SharedDsp::new();
        let mut g = [0.0; 10];
        g[5] = 6.0; // 1 kHz
        shared.set(DspParams { eq_on: true, eq_gains_db: g, ..Default::default() });
        let mut p = Processor::new(48000);
        let mut x = sine(1000.0, 48000.0, 48000);
        let before = rms(&x[48000..]);
        p.process(&shared, &mut x);
        let after = rms(&x[48000..]);
        let db = 20.0 * (after / before).log10();
        assert!((db - 6.0).abs() < 0.7, "got {db} dB");
        // far band barely affected
        let mut y = sine(60.0, 48000.0, 48000);
        let b2 = rms(&y[48000..]);
        let mut p2 = Processor::new(48000);
        p2.process(&shared, &mut y);
        assert!((20.0 * (rms(&y[48000..]) / b2).log10()).abs() < 0.5);
    }

    #[test]
    fn bass_boost_and_balance() {
        let shared = SharedDsp::new();
        shared.set(DspParams { bass_on: true, bass_db: 6.0, balance: 1.0, ..Default::default() });
        let mut p = Processor::new(44100);
        let mut x = sine(40.0, 44100.0, 44100);
        let before = rms(&x[44100..]);
        p.process(&shared, &mut x);
        let left: Vec<f32> = x.iter().step_by(2).copied().collect();
        let right: Vec<f32> = x.iter().skip(1).step_by(2).copied().collect();
        assert!(rms(&left) < 1e-6, "balance full right should mute left");
        assert!(rms(&right[22050..]) > before * 1.6);
    }

    #[test]
    fn resampler_ratio() {
        let mut r = Resampler::new(44100, 48000);
        let mut out = Vec::new();
        let input = sine(440.0, 44100.0, 4410);
        for chunk in input.chunks(882) {
            r.process(chunk, &mut out);
        }
        let frames = out.len() / 2;
        assert!((frames as i64 - 4800).abs() <= 2, "{frames}");
    }

    #[test]
    fn stereo_conversion() {
        let mut o = Vec::new();
        to_stereo(&[1.0, 2.0], 1, &mut o);
        assert_eq!(o, vec![1.0, 1.0, 2.0, 2.0]);
        o.clear();
        to_stereo(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, &mut o);
        assert_eq!(o, vec![1.0, 2.0, 4.0, 5.0]);
    }
}

#[cfg(test)]
mod headroom_tests {
    use super::*;
    #[test]
    fn peak_gain_follows_the_real_curve() {
        assert_eq!(peak_gain_db(&DspParams::default()), 0.0);
        let bass = DspParams { bass_on: true, bass_db: 9.0, ..Default::default() };
        assert!((peak_gain_db(&bass) - 9.0).abs() < 0.3, "{}", peak_gain_db(&bass));
        let mut g = [0.0; 10];
        g[5] = 6.0; // 1 kHz
        let one = DspParams { eq_on: true, eq_gains_db: g, ..Default::default() };
        assert!((peak_gain_db(&one) - 6.0).abs() < 0.3);
        // a cut never needs headroom
        g[5] = -6.0;
        assert!(peak_gain_db(&DspParams { eq_on: true, eq_gains_db: g, ..Default::default() }) < 0.1);
    }
}
