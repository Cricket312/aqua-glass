//! Aqua Glass' own audio processing: preamp → parametric EQ (10 peaking bands +
//! bass low shelf) → balance, plus a linear resampler and channel conversion.
//!
//! The `Processor` runs inside the real-time output callback (after the playback
//! buffer, so a change is heard in the next ≈10 ms, not after the buffered audio):
//!
//! * **No locks, no allocation, no I/O on the audio thread.** The UI publishes
//!   settings through `SharedDsp`, a sequence lock over plain atomics: the audio
//!   thread reads them without ever waiting, and only when they changed.
//! * **Click-free changes.** Gain, frequency and Q glide to new values (≈10 ms
//!   time constant) in 32-frame steps; filter coefficients are recalculated only
//!   for bands that are actually moving. Turning the EQ or a band off fades it to
//!   0 dB first, then the band is skipped entirely.
//! * **Biquads in f64** (RBJ cookbook, transposed direct form II) — stable at
//!   31 Hz / 44.1 kHz where f32 coefficients lose precision. Parameters are
//!   validated, so bad values can't make a filter blow up.
//! * **Bypass costs nothing**: with everything flat and unity gain the buffer is
//!   left untouched (bit-exact).
//!
//! Only audio that Aqua Glass outputs itself passes through here (local files,
//! and Spotify via the optional librespot backend). Spotify Soloist plays its
//! audio itself, so it can't be processed.

use crate::config::EQ_FREQS;
use std::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

/// The settings the EQ page / audio settings expose.
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

// ---------------------------------------------------------------------------
// Filter description

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Peak,
    LowShelf,
}

/// One EQ filter. `enabled = false` (or the EQ being off) fades it to 0 dB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandSpec {
    pub kind: FilterKind,
    pub freq: f32,
    pub q: f32,
    pub gain_db: f32,
    pub enabled: bool,
}

/// Q of the 10 graphic-EQ bands (one octave apart; ≈1 octave bandwidth).
pub const EQ_Q: f32 = 1.41;
/// Bass boost: low shelf at 120 Hz with a Butterworth slope.
pub const BASS_FREQ: f32 = 120.0;
pub const BASS_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;
pub const MAX_BANDS: usize = 11;
const MAX_GAIN_DB: f32 = 24.0;

impl BandSpec {
    /// Clamp to values that give a stable, meaningful filter at `rate`.
    pub fn sanitized(self, rate: f32) -> BandSpec {
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        BandSpec {
            kind: self.kind,
            freq: fin(self.freq, 1000.0).clamp(10.0, 0.45 * rate),
            q: fin(self.q, 0.707).clamp(0.1, 24.0),
            gain_db: fin(self.gain_db, 0.0).clamp(-MAX_GAIN_DB, MAX_GAIN_DB),
            enabled: self.enabled,
        }
    }
    /// Gain this band is heading for (0 dB when off).
    fn target_db(&self) -> f32 {
        if self.enabled && self.gain_db.abs() > 0.05 {
            self.gain_db
        } else {
            0.0
        }
    }
}

/// The native filter layout for the EQ page's settings: 10 peaking bands at
/// `EQ_FREQS` and the bass-boost shelf (same shapes the UI has always used).
pub fn bands(p: &DspParams) -> [BandSpec; MAX_BANDS] {
    let mut out = [BandSpec { kind: FilterKind::Peak, freq: 1000.0, q: EQ_Q, gain_db: 0.0, enabled: false }; MAX_BANDS];
    for (i, f) in EQ_FREQS.iter().enumerate() {
        out[i] = BandSpec { kind: FilterKind::Peak, freq: *f, q: EQ_Q, gain_db: p.eq_gains_db[i], enabled: p.eq_on };
    }
    out[10] = BandSpec { kind: FilterKind::LowShelf, freq: BASS_FREQ, q: BASS_Q, gain_db: p.bass_db, enabled: p.bass_on };
    out
}

// ---------------------------------------------------------------------------
// UI → audio thread hand-over (lock-free for the reader)

const WORDS: usize = 15; // eq_on, 10 gains, bass_on, bass_db, preamp_db, balance

/// Settings shared with the audio callback: a sequence lock over atomics. The
/// writer (UI thread) never blocks the reader (audio thread); a read that
/// overlaps a write is simply retried on the next callback.
pub struct SharedDsp {
    seq: AtomicU64,
    words: [AtomicU32; WORDS],
    writer: Mutex<()>,
}

fn encode(p: &DspParams) -> [u32; WORDS] {
    let mut w = [0u32; WORDS];
    w[0] = p.eq_on as u32;
    for i in 0..10 {
        w[1 + i] = p.eq_gains_db[i].to_bits();
    }
    w[11] = p.bass_on as u32;
    w[12] = p.bass_db.to_bits();
    w[13] = p.preamp_db.to_bits();
    w[14] = p.balance.to_bits();
    w
}

fn decode(w: &[u32; WORDS]) -> DspParams {
    let mut g = [0f32; 10];
    for (i, v) in g.iter_mut().enumerate() {
        *v = f32::from_bits(w[1 + i]);
    }
    DspParams {
        eq_on: w[0] != 0,
        eq_gains_db: g,
        bass_on: w[11] != 0,
        bass_db: f32::from_bits(w[12]),
        preamp_db: f32::from_bits(w[13]),
        balance: f32::from_bits(w[14]),
    }
}

impl SharedDsp {
    pub fn new() -> Self {
        let w = encode(&DspParams::default());
        SharedDsp { seq: AtomicU64::new(2), words: w.map(AtomicU32::new), writer: Mutex::new(()) }
    }

    /// Publish new settings (UI thread). No-op when nothing changed.
    pub fn set(&self, p: DspParams) {
        let _g = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let w = encode(&p);
        if self.words.iter().zip(w.iter()).all(|(a, b)| a.load(Ordering::Relaxed) == *b) {
            return;
        }
        self.seq.fetch_add(1, Ordering::Relaxed); // odd: write in progress
        fence(Ordering::Release);
        for (a, b) in self.words.iter().zip(w.iter()) {
            a.store(*b, Ordering::Relaxed);
        }
        self.seq.fetch_add(1, Ordering::Release); // even: consistent again
    }

    /// Settings newer than `seen`, if any and if not mid-write. Never blocks.
    fn read_if_newer(&self, seen: u64) -> Option<(u64, DspParams)> {
        let s1 = self.seq.load(Ordering::Acquire);
        if s1 == seen || s1 & 1 == 1 {
            return None;
        }
        let mut w = [0u32; WORDS];
        for (o, a) in w.iter_mut().zip(self.words.iter()) {
            *o = a.load(Ordering::Relaxed);
        }
        fence(Ordering::Acquire);
        (self.seq.load(Ordering::Relaxed) == s1).then(|| (s1, decode(&w)))
    }

    /// Current settings (for tests and the UI).
    pub fn get(&self) -> DspParams {
        let _g = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let mut w = [0u32; WORDS];
        for (o, a) in w.iter_mut().zip(self.words.iter()) {
            *o = a.load(Ordering::Relaxed);
        }
        decode(&w)
    }
}

impl Default for SharedDsp {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Biquad

#[derive(Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: [f64; 2],
    z2: [f64; 2],
}

impl Default for Biquad {
    fn default() -> Self {
        Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: [0.0; 2], z2: [0.0; 2] }
    }
}

impl Biquad {
    /// RBJ "Audio EQ Cookbook" coefficients; identity if anything is off.
    fn design(&mut self, kind: FilterKind, rate: f32, freq: f32, q: f32, gain_db: f32) {
        let a = 10f64.powf(gain_db as f64 / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * (freq as f64 / rate as f64);
        let (s, c) = w0.sin_cos();
        let alpha = s / (2.0 * q as f64);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::Peak => (1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a),
            FilterKind::LowShelf => {
                let sa = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * c + sa),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                    a * ((a + 1.0) - (a - 1.0) * c - sa),
                    (a + 1.0) + (a - 1.0) * c + sa,
                    -2.0 * ((a - 1.0) + (a + 1.0) * c),
                    (a + 1.0) + (a - 1.0) * c - sa,
                )
            }
        };
        let n = [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0];
        // stability: poles inside the unit circle (|a2| < 1, |a1| < 1 + a2)
        if n.iter().all(|v| v.is_finite()) && n[4].abs() < 1.0 && n[3].abs() < 1.0 + n[4] {
            [self.b0, self.b1, self.b2, self.a1, self.a2] = n;
        } else {
            [self.b0, self.b1, self.b2, self.a1, self.a2] = [1.0, 0.0, 0.0, 0.0, 0.0];
        }
    }

    /// |H(e^jw)| at angular frequency `w` (radians/sample).
    fn magnitude(&self, w: f64) -> f64 {
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let num = ((self.b0 + self.b1 * c1 + self.b2 * c2).powi(2) + (self.b1 * s1 + self.b2 * s2).powi(2)).sqrt();
        let den = ((1.0 + self.a1 * c1 + self.a2 * c2).powi(2) + (self.a1 * s1 + self.a2 * s2).powi(2)).sqrt();
        num / den.max(1e-12)
    }

    /// Filter interleaved stereo in place.
    #[inline]
    fn run_stereo(&mut self, buf: &mut [f32]) {
        let (b0, b1, b2, a1, a2) = (self.b0, self.b1, self.b2, self.a1, self.a2);
        let (mut z1, mut z2) = (self.z1, self.z2);
        for fr in buf.chunks_exact_mut(2) {
            for ch in 0..2 {
                let x = fr[ch] as f64;
                let y = b0 * x + z1[ch];
                z1[ch] = b1 * x - a1 * y + z2[ch];
                z2[ch] = b2 * x - a2 * y;
                fr[ch] = y as f32;
            }
        }
        // flush values that would decay into denormals during silence (slow on x86)
        for z in z1.iter_mut().chain(z2.iter_mut()) {
            if z.abs() < 1e-25 {
                *z = 0.0;
            }
        }
        self.z1 = z1;
        self.z2 = z2;
    }

    fn reset(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }
}

/// The combined EQ + bass curve's gain (dB) at `freq` Hz (at 48 kHz output).
pub fn response_db(p: &DspParams, freq: f32) -> f32 {
    response_db_at(p, freq, 48_000.0)
}

/// The curve as processed at sample rate `rate`.
pub fn response_db_at(p: &DspParams, freq: f32, rate: f32) -> f32 {
    let w = 2.0 * std::f64::consts::PI * freq as f64 / rate as f64;
    let mut g = 1.0;
    for b in bands(p).iter().map(|b| b.sanitized(rate)) {
        let t = b.target_db();
        if t != 0.0 {
            let mut bq = Biquad::default();
            bq.design(b.kind, rate, b.freq, b.q, t);
            g *= bq.magnitude(w);
        }
    }
    (20.0 * g.log10()) as f32
}

/// Highest gain (dB) the EQ + bass curve reaches anywhere in 20 Hz–20 kHz, from
/// the real filter curves (bands overlap and partly cancel, so adding up the
/// largest boosts would overstate it). 0 when nothing boosts.
pub fn peak_gain_db(p: &DspParams) -> f32 {
    if bands(p).iter().all(|b| b.target_db() == 0.0) {
        return 0.0;
    }
    (0..400)
        .map(|i| response_db(p, 20.0 * 1000f32.powf(i as f32 / 399.0))) // log-spaced
        .fold(f32::MIN, f32::max)
}

// ---------------------------------------------------------------------------
// Real-time processor

/// Frames per smoothing step: parameters move and coefficients are refreshed at
/// most this often (0.7 ms at 48 kHz).
const STEP: usize = 32;
/// Glide time constant for parameter changes.
const SMOOTH_S: f32 = 0.010;

#[derive(Clone, Copy)]
struct Band {
    target: BandSpec,
    /// Current (gliding) gain, log2(frequency) and Q.
    gain: f32,
    lfreq: f32,
    q: f32,
    /// Values the coefficients were computed for.
    designed: (f32, f32, f32),
    active: bool,
    bq: Biquad,
}

fn glide(cur: &mut f32, target: f32, k: f32, snap: f32) -> bool {
    if *cur == target {
        return false;
    }
    let d = target - *cur;
    *cur = if d.abs() <= snap { target } else { *cur + d * k };
    true
}

/// Per-stream processing state (one per output stream, at the device rate).
/// ≈ 0.7 KB, allocation-free.
pub struct Processor {
    rate: f32,
    seen: u64,
    primed: bool,
    k: f32,
    bands: [Band; MAX_BANDS],
    /// Linear output gain per channel (preamp × balance): target and current.
    gain_target: [f32; 2],
    gain: [f32; 2],
    /// Peaks above 0.95 get a gentle soft clip while anything boosts.
    boosting: bool,
}

impl Processor {
    pub fn new(rate: u32) -> Self {
        let rate = rate.max(8_000) as f32;
        let b = Band {
            target: BandSpec { kind: FilterKind::Peak, freq: 1000.0, q: EQ_Q, gain_db: 0.0, enabled: false },
            gain: 0.0,
            lfreq: 1000f32.log2(),
            q: EQ_Q,
            designed: (f32::NAN, 0.0, 0.0),
            active: false,
            bq: Biquad::default(),
        };
        Processor {
            rate,
            seen: 0,
            primed: false,
            k: 1.0 - (-(STEP as f32) / (SMOOTH_S * rate)).exp(),
            bands: [b; MAX_BANDS],
            gain_target: [1.0; 2],
            gain: [1.0; 2],
            boosting: false,
        }
    }

    /// Pick up new settings (if any) without waiting.
    fn refresh(&mut self, shared: &SharedDsp) {
        let Some((seq, p)) = shared.read_if_newer(self.seen) else { return };
        self.seen = seq;
        for (band, spec) in self.bands.iter_mut().zip(bands(&p)) {
            band.target = spec.sanitized(self.rate);
        }
        let pre = 10f32.powf(p.preamp_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB) / 20.0);
        let bal = if p.balance.is_finite() { p.balance.clamp(-1.0, 1.0) } else { 0.0 };
        let pre = if pre.is_finite() { pre } else { 1.0 };
        self.gain_target = [pre * (1.0 - bal).min(1.0), pre * (1.0 + bal).min(1.0)];
        if !self.primed {
            // new stream (start-up, device change): start at the settings, no glide
            self.primed = true;
            self.gain = self.gain_target;
            for b in self.bands.iter_mut() {
                b.gain = b.target.target_db();
                b.lfreq = b.target.freq.log2();
                b.q = b.target.q;
            }
        }
    }

    /// Move every parameter one step towards its target; recompute coefficients
    /// only for bands that changed.
    fn advance(&mut self) {
        let (k, rate) = (self.k, self.rate);
        let mut boosting = false;
        for b in self.bands.iter_mut() {
            let t = b.target;
            let mut moved = glide(&mut b.gain, t.target_db(), k, 0.01);
            moved |= glide(&mut b.lfreq, t.freq.log2(), k, 0.001);
            moved |= glide(&mut b.q, t.q, k, 0.001);
            let want_active = b.gain != 0.0;
            if want_active && (moved || !b.active || b.designed != (b.gain, b.lfreq, b.q)) {
                b.bq.design(t.kind, rate, b.lfreq.exp2(), b.q, b.gain);
                b.designed = (b.gain, b.lfreq, b.q);
            }
            if b.active && !want_active {
                b.bq.reset(); // at exactly 0 dB the filter was already a pass-through
            }
            b.active = want_active;
            boosting |= b.gain > 0.0;
        }
        self.boosting = boosting || self.gain_target.iter().chain(self.gain.iter()).any(|g| *g > 1.0);
    }

    fn is_idle(&self) -> bool {
        self.gain == [1.0; 2] && self.gain_target == [1.0; 2] && self.bands.iter().all(|b| !b.active && b.target.target_db() == 0.0)
    }

    /// Process interleaved stereo in place (real-time safe).
    pub fn process(&mut self, shared: &SharedDsp, buf: &mut [f32]) {
        self.refresh(shared);
        if self.is_idle() {
            return; // bypass: untouched, bit-exact
        }
        for block in buf.chunks_mut(STEP * 2) {
            self.advance();
            for b in self.bands.iter_mut().filter(|b| b.active) {
                b.bq.run_stereo(block);
            }
            // preamp × balance, ramped linearly across the block
            let from = self.gain;
            for (c, g) in self.gain.iter_mut().enumerate() {
                glide(g, self.gain_target[c], self.k, 1e-5);
            }
            let to = self.gain;
            if from == [1.0; 2] && to == [1.0; 2] {
                // unity: nothing to multiply
            } else if from == to {
                for fr in block.chunks_exact_mut(2) {
                    fr[0] *= to[0];
                    fr[1] *= to[1];
                }
            } else {
                let n = (block.len() / 2).max(1) as f32;
                for (i, fr) in block.chunks_exact_mut(2).enumerate() {
                    let t = (i + 1) as f32 / n;
                    fr[0] *= from[0] + (to[0] - from[0]) * t;
                    fr[1] *= from[1] + (to[1] - from[1]) * t;
                }
            }
            if self.boosting {
                // gentle soft clip of peaks above 0.95 only (below that: untouched)
                for s in block.iter_mut() {
                    if s.abs() > 0.95 {
                        *s = s.signum() * (0.95 + (s.abs() - 0.95).tanh() * 0.05);
                    }
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

    fn db(a: f32, b: f32) -> f32 {
        20.0 * (a / b).log10()
    }

    fn eq_at(band: usize, g: f32) -> DspParams {
        let mut gains = [0.0; 10];
        gains[band] = g;
        DspParams { eq_on: true, eq_gains_db: gains, ..Default::default() }
    }

    /// Run `x` through a fresh processor in 480-frame callbacks.
    fn run(shared: &SharedDsp, rate: u32, x: &mut [f32]) {
        let mut p = Processor::new(rate);
        for c in x.chunks_mut(960) {
            p.process(shared, c);
        }
    }

    #[test]
    fn flat_is_bit_exact() {
        let shared = SharedDsp::new();
        let mut x = sine(440.0, 44100.0, 1000);
        let orig = x.clone();
        run(&shared, 44100, &mut x);
        assert_eq!(x, orig);
    }

    #[test]
    fn eq_boosts_band() {
        let shared = SharedDsp::new();
        shared.set(eq_at(5, 6.0)); // 1 kHz
        let mut x = sine(1000.0, 48000.0, 48000);
        let before = rms(&x[48000..]);
        run(&shared, 48000, &mut x);
        let d = db(rms(&x[48000..]), before);
        assert!((d - 6.0).abs() < 0.3, "got {d} dB");
        // far band barely affected
        let mut y = sine(60.0, 48000.0, 48000);
        let b2 = rms(&y[48000..]);
        run(&shared, 48000, &mut y);
        assert!(db(rms(&y[48000..]), b2).abs() < 0.5);
    }

    /// The measured gain matches the curve `response_db` reports, at every rate.
    #[test]
    fn measured_response_matches_curve_at_common_rates() {
        let p = DspParams { eq_on: true, eq_gains_db: [3.0, -2.0, 4.0, 0.0, -6.0, 2.0, 0.0, 5.0, -3.0, 6.0], bass_on: true, bass_db: 6.0, ..Default::default() };
        let shared = SharedDsp::new();
        shared.set(p.clone());
        for rate in [44_100u32, 48_000, 96_000] {
            for f in [40.0, 125.0, 500.0, 1000.0, 4000.0, 12_000.0] {
                let mut x = sine(f, rate as f32, rate as usize);
                let before = rms(&x[rate as usize..]);
                run(&shared, rate, &mut x);
                let got = db(rms(&x[rate as usize..]), before);
                let want = response_db_at(&p, f, rate as f32);
                assert!((got - want).abs() < 0.1, "{rate} Hz rate, {f} Hz: measured {got:.2} dB, curve {want:.2} dB");
                assert!(x.iter().all(|v| v.is_finite()));
            }
        }
    }

    #[test]
    fn bass_boost_and_balance() {
        let shared = SharedDsp::new();
        shared.set(DspParams { bass_on: true, bass_db: 6.0, balance: 1.0, ..Default::default() });
        let mut x = sine(40.0, 44100.0, 44100);
        let before = rms(&x[44100..]);
        run(&shared, 44100, &mut x);
        let left: Vec<f32> = x.iter().step_by(2).copied().collect();
        let right: Vec<f32> = x.iter().skip(1).step_by(2).copied().collect();
        assert!(rms(&left) < 1e-6, "balance full right should mute left");
        assert!(rms(&right[22050..]) > before * 1.6);
    }

    /// Channels are filtered independently: a left-only signal stays left-only.
    #[test]
    fn stereo_channels_stay_separate() {
        let shared = SharedDsp::new();
        shared.set(DspParams { eq_on: true, eq_gains_db: [6.0; 10], bass_on: true, bass_db: 9.0, preamp_db: -6.0, ..Default::default() });
        let mut x = sine(200.0, 48000.0, 9600);
        for fr in x.chunks_exact_mut(2) {
            fr[1] = 0.0;
        }
        run(&shared, 48000, &mut x);
        assert!(x.iter().skip(1).step_by(2).all(|v| *v == 0.0));
        assert!(rms(&x) > 0.01);
    }

    /// Changes glide instead of jumping: no step in the waveform (a click).
    #[test]
    fn parameter_changes_are_click_free() {
        let shared = SharedDsp::new();
        shared.set(DspParams { preamp_db: -18.0, ..Default::default() });
        let mut p = Processor::new(48000);
        let mut x = sine(1000.0, 48000.0, 48000);
        let natural = 0.25 * 2.0 * std::f32::consts::PI * 1000.0 / 48000.0; // largest step of the sine itself
        let mut prev = 0.0;
        let mut worst: f32 = 0.0;
        for (i, c) in x.chunks_mut(960).enumerate() {
            match i {
                10 => shared.set(DspParams { preamp_db: -6.0, ..eq_at(5, 12.0) }), // +24 dB at 1 kHz in one go
                20 => shared.set(DspParams { preamp_db: -18.0, ..Default::default() }), // EQ off again
                _ => {}
            }
            p.process(&shared, c);
            for v in c.iter().step_by(2) {
                worst = worst.max((v - prev).abs());
                prev = *v;
            }
        }
        // the level rises ×16 (0.03 → 0.5 peak, below the soft clip); jumping there
        // at once would step by up to ≈0.47 — gliding keeps every step at the sine's own
        let louder = natural * 0.5 / 0.25;
        assert!(worst < louder * 1.1, "largest step {worst} vs the louder sine's own {louder}");
    }

    /// Turning the EQ off fades to flat, then the processor is idle and bit-exact again.
    #[test]
    fn bypass_fades_out_then_costs_nothing() {
        let shared = SharedDsp::new();
        shared.set(eq_at(3, 9.0));
        let mut p = Processor::new(48000);
        let mut x = sine(250.0, 48000.0, 4800);
        p.process(&shared, &mut x);
        assert!(!p.is_idle());
        shared.set(DspParams { eq_on: false, ..eq_at(3, 9.0) });
        let mut y = sine(250.0, 48000.0, 9600); // 200 ms
        p.process(&shared, &mut y);
        assert!(p.is_idle(), "faded out within 200 ms");
        let mut z = sine(250.0, 48000.0, 480);
        let orig = z.clone();
        p.process(&shared, &mut z);
        assert_eq!(z, orig);
    }

    /// A new stream (start-up or another output device) starts at the current settings.
    #[test]
    fn new_stream_starts_at_settings_without_glide() {
        let shared = SharedDsp::new();
        shared.set(DspParams { preamp_db: 6.0206, ..Default::default() });
        let mut x = vec![0.1f32; 64];
        Processor::new(44100).process(&shared, &mut x);
        assert!((x[0] - 0.2).abs() < 1e-4, "{}", x[0]);
    }

    #[test]
    fn invalid_values_cannot_destabilise_filters() {
        let shared = SharedDsp::new();
        shared.set(DspParams { eq_on: true, eq_gains_db: [f32::NAN, f32::INFINITY, -1e9, 1e9, 0.0, 0.0, 0.0, 0.0, 0.0, 99.0], bass_on: true, bass_db: f32::NAN, preamp_db: f32::INFINITY, balance: f32::NAN });
        let mut x = sine(100.0, 48000.0, 48000);
        run(&shared, 48000, &mut x);
        assert!(x.iter().all(|v| v.is_finite() && v.abs() <= 1.0), "finite and soft-clipped");
        // out-of-range frequency / Q are clamped
        let b = BandSpec { kind: FilterKind::Peak, freq: 40_000.0, q: 0.0, gain_db: 6.0, enabled: true }.sanitized(44_100.0);
        assert!(b.freq < 22_050.0 && b.q >= 0.1);
        let mut bq = Biquad::default();
        bq.design(FilterKind::Peak, 44_100.0, b.freq, b.q, b.gain_db);
        assert!(bq.a2.abs() < 1.0);
    }

    /// The audio thread never sees half of one update and half of another.
    #[test]
    fn shared_settings_are_read_consistently_without_locking() {
        let shared = std::sync::Arc::new(SharedDsp::new());
        let w = shared.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let st = stop.clone();
        let writer = std::thread::spawn(move || {
            let mut i = 0u32;
            while !st.load(Ordering::Relaxed) {
                let g = (i % 25) as f32 - 12.0;
                w.set(DspParams { eq_on: true, eq_gains_db: [g; 10], bass_on: true, bass_db: g, preamp_db: g, balance: 0.0 });
                i += 1;
            }
        });
        let mut seen = 2; // the defaults, published before the writer starts
        let mut reads = 0;
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(300) {
            if let Some((s, p)) = shared.read_if_newer(seen) {
                seen = s;
                reads += 1;
                let g = p.eq_gains_db[0];
                assert!(p.eq_gains_db.iter().all(|v| *v == g) && p.bass_db == g && p.preamp_db == g, "torn read: {p:?}");
            }
        }
        stop.store(true, Ordering::Relaxed);
        writer.join().unwrap();
        assert!(reads > 10, "{reads}");
    }

    #[test]
    fn unchanged_settings_are_not_republished() {
        let s = SharedDsp::new();
        let v = s.seq.load(Ordering::Relaxed);
        s.set(DspParams::default());
        assert_eq!(s.seq.load(Ordering::Relaxed), v);
        s.set(eq_at(0, 1.0));
        assert_eq!(s.seq.load(Ordering::Relaxed), v + 2);
        assert_eq!(s.get(), eq_at(0, 1.0));
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

    #[test]
    fn peak_gain_follows_the_real_curve() {
        assert_eq!(peak_gain_db(&DspParams::default()), 0.0);
        let bass = DspParams { bass_on: true, bass_db: 9.0, ..Default::default() };
        assert!((peak_gain_db(&bass) - 9.0).abs() < 0.3, "{}", peak_gain_db(&bass));
        assert!((peak_gain_db(&eq_at(5, 6.0)) - 6.0).abs() < 0.3);
        // a cut never needs headroom
        assert!(peak_gain_db(&eq_at(5, -6.0)) < 0.1);
        // overlapping bands: less than the sum of the largest boosts
        let vocal = [-2.0, -1.0, 0.0, 1.5, 3.0, 3.0, 2.0, 1.0, 0.0, -1.0];
        let p = DspParams { eq_on: true, eq_gains_db: vocal, bass_on: true, bass_db: 14.5, ..Default::default() };
        let peak = peak_gain_db(&p);
        assert!(peak < 16.0 && peak > 12.0, "{peak}");
    }
}
