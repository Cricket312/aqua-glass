//! FFT analysis for the visualizer. Only runs while a visualizer is on screen;
//! when hidden or disabled the output tap is switched off and no FFT is computed.

use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::sync::Arc;

pub const FFT_SIZE: usize = 2048;

pub struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    pub bars: Vec<f32>,
    pub wave: Vec<f32>,
}

impl Analyzer {
    pub fn new() -> Analyzer {
        let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32).cos())
            .collect();
        let sl = fft.get_inplace_scratch_len();
        Analyzer { fft, window, buf: vec![Complex::default(); FFT_SIZE], scratch: vec![Complex::default(); sl], bars: Vec::new(), wave: Vec::new() }
    }

    /// Update bar heights (0..1) from mono samples using log-spaced bands.
    pub fn update(&mut self, samples: &[f32], rate: u32, nbars: usize, sensitivity: f32, smoothing: f32) {
        self.bars.resize(nbars, 0.0);
        self.wave.clear();
        let step = (samples.len() / 256).max(1);
        self.wave.extend(samples.iter().step_by(step).take(256));
        if samples.len() < FFT_SIZE {
            for b in self.bars.iter_mut() {
                *b *= smoothing;
            }
            return;
        }
        let tail = &samples[samples.len() - FFT_SIZE..];
        for (i, s) in tail.iter().enumerate() {
            self.buf[i] = Complex::new(s * self.window[i], 0.0);
        }
        self.fft.process_with_scratch(&mut self.buf, &mut self.scratch);
        let nyq = rate as f32 / 2.0;
        let (fmin, fmax) = (30.0f32, 16000.0f32.min(nyq));
        let bin_hz = rate as f32 / FFT_SIZE as f32;
        for b in 0..nbars {
            let f0 = fmin * (fmax / fmin).powf(b as f32 / nbars as f32);
            let f1 = fmin * (fmax / fmin).powf((b + 1) as f32 / nbars as f32);
            let i0 = ((f0 / bin_hz) as usize).max(1);
            let i1 = ((f1 / bin_hz) as usize).max(i0 + 1).min(FFT_SIZE / 2);
            let mut m = 0.0f32;
            for i in i0..i1 {
                m = m.max(self.buf[i].norm());
            }
            let db = 20.0 * (m / (FFT_SIZE as f32 / 4.0) + 1e-9).log10();
            let v = ((db + 60.0) / 60.0 * sensitivity).clamp(0.0, 1.0);
            let old = self.bars[b];
            self.bars[b] = if v > old { v * (1.0 - smoothing * 0.5) + old * smoothing * 0.5 } else { old * smoothing + v * (1.0 - smoothing) };
        }
    }
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn peak_in_right_band() {
        let rate = 48000;
        let s: Vec<f32> = (0..FFT_SIZE * 2).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / rate as f32).sin() * 0.5).collect();
        let mut a = Analyzer::new();
        for _ in 0..10 {
            a.update(&s, rate, 32, 1.0, 0.0);
        }
        let (imax, _) = a.bars.iter().enumerate().fold((0, 0.0), |acc, (i, v)| if *v > acc.1 { (i, *v) } else { acc });
        // 1 kHz on a 30 Hz..16 kHz log scale with 32 bands lands near band 17-18
        let expected = (32.0 * (1000f32 / 30.0).ln() / (16000f32 / 30.0).ln()) as usize;
        assert!((imax as i32 - expected as i32).abs() <= 1, "{imax} vs {expected}");
        a.update(&[], rate, 32, 1.0, 0.5);
    }
}
