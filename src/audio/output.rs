//! The single audio output pipeline (cpal → WASAPI shared mode on Windows).
//!
//! Producers (librespot sink, local-file decoder) push interleaved stereo f32
//! at the device rate into a bounded ring buffer. The real-time callback only
//! copies samples, applies volume/fades and, when the visualizer is visible,
//! copies a mono tap for the FFT. Device loss is detected via the error
//! callback and the stream is reopened on the default device.

use super::dsp::{Processor, Resampler, SharedDsp};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

pub const VIS_LEN: usize = 2048;

fn f32_load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}
fn f32_store(a: &AtomicU32, v: f32) {
    a.store(v.to_bits(), Ordering::Relaxed)
}

pub struct Shared {
    ring: Mutex<VecDeque<f32>>,
    /// Capacity in samples.
    capacity: AtomicU64,
    volume: AtomicU32,
    muted: AtomicBool,
    /// Gain target used for fades (0 or 1) and per-sample step.
    fade_target: AtomicU32,
    fade_ms: AtomicU32,
    paused: AtomicBool,
    /// Frames consumed by the device / pushed by producers since start.
    pub consumed: AtomicU64,
    pub pushed: AtomicU64,
    pub device_rate: AtomicU32,
    pub vis_enabled: AtomicBool,
    pub vis: Mutex<VecDeque<f32>>,
    /// Bumped by `flush`; producers holding an older generation drop their data.
    pub generation: AtomicU64,
    pub dsp: SharedDsp,
    pub device_error: Mutex<Option<String>>,
    pub device_name: Mutex<String>,
    current_gain: Mutex<f32>,
}

#[derive(Clone)]
pub struct Output {
    pub s: Arc<Shared>,
    cmd: mpsc::Sender<OutCmd>,
}

enum OutCmd {
    Open(String),
}

impl Output {
    pub fn start(device: &str, buffer_ms: u32) -> Output {
        let s = Arc::new(Shared {
            ring: Mutex::new(VecDeque::with_capacity(48000)),
            capacity: AtomicU64::new(48_000),
            volume: AtomicU32::new(0.7f32.to_bits()),
            muted: AtomicBool::new(false),
            fade_target: AtomicU32::new(1.0f32.to_bits()),
            fade_ms: AtomicU32::new(0),
            paused: AtomicBool::new(true),
            consumed: AtomicU64::new(0),
            pushed: AtomicU64::new(0),
            device_rate: AtomicU32::new(48_000),
            vis_enabled: AtomicBool::new(false),
            vis: Mutex::new(VecDeque::new()),
            generation: AtomicU64::new(0),
            dsp: SharedDsp::new(),
            device_error: Mutex::new(None),
            device_name: Mutex::new(String::new()),
            current_gain: Mutex::new(1.0),
        });
        let (tx, rx) = mpsc::channel();
        let out = Output { s: s.clone(), cmd: tx };
        out.set_buffer_ms(buffer_ms);
        let dev = device.to_string();
        std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || output_thread(s, rx, dev))
            .expect("spawn audio thread");
        // give the stream a moment to report its rate
        for _ in 0..50 {
            if !out.s.device_name.lock().unwrap().is_empty() || out.s.device_error.lock().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        out
    }

    pub fn open_device(&self, name: &str) {
        let _ = self.cmd.send(OutCmd::Open(name.to_string()));
    }

    pub fn rate(&self) -> u32 {
        self.s.device_rate.load(Ordering::Relaxed)
    }

    pub fn set_buffer_ms(&self, ms: u32) {
        let rate = self.rate() as u64;
        self.s.capacity.store((rate * ms as u64 / 1000).max(2048) * 2, Ordering::Relaxed);
    }

    pub fn set_volume(&self, v: f32) {
        f32_store(&self.s.volume, v.clamp(0.0, 1.0));
    }
    pub fn set_muted(&self, m: bool) {
        self.s.muted.store(m, Ordering::Relaxed);
    }
    pub fn set_fade_ms(&self, ms: u32) {
        self.s.fade_ms.store(ms, Ordering::Relaxed);
    }
    pub fn fade_ms(&self) -> u32 {
        self.s.fade_ms.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, p: bool) {
        if p {
            f32_store(&self.s.fade_target, 0.0);
            if self.fade_ms() == 0 {
                self.s.paused.store(true, Ordering::Relaxed);
            }
        } else {
            self.s.paused.store(false, Ordering::Relaxed);
            f32_store(&self.s.fade_target, 1.0);
        }
    }
    pub fn is_paused(&self) -> bool {
        self.s.paused.load(Ordering::Relaxed)
    }

    /// Wait for a fade-out to complete (bounded).
    pub fn wait_faded(&self) {
        let ms = self.fade_ms();
        if ms == 0 {
            return;
        }
        let deadline = std::time::Instant::now() + Duration::from_millis(ms as u64 + 100);
        while std::time::Instant::now() < deadline && *self.s.current_gain.lock().unwrap() > 0.001 {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Drop buffered audio (seek/skip). Returns the new generation.
    pub fn flush(&self) -> u64 {
        let g = self.s.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let mut r = self.s.ring.lock().unwrap();
        let n = r.len() as u64 / 2;
        r.clear();
        // dropped frames count as never pushed so positions stay consistent
        self.s.pushed.fetch_sub(n.min(self.s.pushed.load(Ordering::Relaxed)), Ordering::Relaxed);
        g
    }

    pub fn generation(&self) -> u64 {
        self.s.generation.load(Ordering::Acquire)
    }

    pub fn buffered_frames(&self) -> u64 {
        self.s.ring.lock().unwrap().len() as u64 / 2
    }

    /// Blocking push of device-rate stereo samples. Returns false if the generation changed (flushed).
    pub fn push(&self, gen: u64, mut data: &[f32]) -> bool {
        while !data.is_empty() {
            if self.generation() != gen {
                return false;
            }
            let cap = self.s.capacity.load(Ordering::Relaxed) as usize;
            let room = {
                let mut r = self.s.ring.lock().unwrap();
                if self.generation() != gen {
                    return false;
                }
                let room = cap.saturating_sub(r.len()) & !1;
                let n = room.min(data.len());
                r.extend(&data[..n]);
                self.s.pushed.fetch_add(n as u64 / 2, Ordering::Relaxed);
                data = &data[n..];
                n
            };
            if room == 0 || !data.is_empty() {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        true
    }

    pub fn set_vis(&self, on: bool) {
        self.s.vis_enabled.store(on, Ordering::Relaxed);
        if !on {
            let mut v = self.s.vis.lock().unwrap();
            v.clear();
            v.shrink_to_fit();
        }
    }

    pub fn vis_snapshot(&self, out: &mut Vec<f32>) {
        out.clear();
        if let Ok(v) = self.s.vis.try_lock() {
            out.extend(v.iter());
        }
    }
}

/// Producer helper: converts arbitrary-rate/channels f32 to the device format,
/// runs DSP, pushes. One per playing source.
pub struct Feeder {
    out: Output,
    gen: u64,
    resampler: Resampler,
    proc: Processor,
    rate: u32,
    dev_rate: u32,
    stereo: Vec<f32>,
    resampled: Vec<f32>,
}

impl Feeder {
    pub fn new(out: &Output, src_rate: u32, gen: u64) -> Feeder {
        let rate = out.rate();
        Feeder {
            out: out.clone(),
            gen,
            resampler: Resampler::new(src_rate, rate),
            proc: Processor::new(rate),
            rate: src_rate,
            dev_rate: rate,
            stereo: Vec::new(),
            resampled: Vec::new(),
        }
    }
    pub fn set_track_gain(&mut self, g: f32) {
        self.proc.track_gain = g;
    }
    pub fn src_rate(&self) -> u32 {
        self.rate
    }
    pub fn set_generation(&mut self, g: u64) {
        self.gen = g;
    }
    /// Returns false when flushed (caller should stop producing for this generation).
    pub fn write(&mut self, samples: &[f32], channels: usize) -> bool {
        // Device rate may change if the device was reopened.
        let dev = self.out.rate();
        if dev != self.dev_rate {
            self.dev_rate = dev;
            self.resampler = Resampler::new(self.rate, dev);
            let g = self.proc.track_gain;
            self.proc = Processor::new(dev);
            self.proc.track_gain = g;
        }
        self.stereo.clear();
        super::dsp::to_stereo(samples, channels, &mut self.stereo);
        self.resampled.clear();
        self.resampler.process(&self.stereo, &mut self.resampled);
        self.proc.process(&self.out.s.dsp, &mut self.resampled);
        let gen = self.gen;
        let out = self.out.clone();
        out.push(gen, &self.resampled)
    }
}

pub fn list_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices().map(|d| d.map(|d| d.to_string()).collect()).unwrap_or_default()
}

fn output_thread(s: Arc<Shared>, rx: mpsc::Receiver<OutCmd>, mut device: String) {
    loop {
        let stream = open_stream(&s, &device);
        let stream = match stream {
            Ok(st) => {
                *s.device_error.lock().unwrap() = None;
                Some(st)
            }
            Err(e) => {
                *s.device_error.lock().unwrap() = Some(e.to_string());
                if s.device_name.lock().unwrap().is_empty() {
                    *s.device_name.lock().unwrap() = "(no device)".into();
                }
                None
            }
        };
        // Wait for a command or a device error (poll the error flag every 500 ms).
        loop {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(OutCmd::Open(name)) => {
                    device = name;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let err = s.device_error.lock().unwrap().is_some();
                    if stream.is_none() || err {
                        // device lost: retry (falls back to default device)
                        std::thread::sleep(Duration::from_millis(1000));
                        if stream.is_some() {
                            device.clear();
                        }
                        break;
                    }
                }
            }
        }
        drop(stream);
    }
}

fn open_stream(s: &Arc<Shared>, device: &str) -> anyhow::Result<cpal::Stream> {
    let host = cpal::default_host();
    let dev = if device.is_empty() {
        host.default_output_device()
    } else {
        host.output_devices()?.find(|d| d.to_string() == device).or_else(|| host.default_output_device())
    }
    .ok_or_else(|| anyhow::anyhow!("No audio output device found"))?;
    let supported = dev.default_output_config()?;
    let fmt = supported.sample_format();
    let mut config = supported.config();
    let channels = config.channels as usize;
    config.buffer_size = cpal::BufferSize::Default;
    s.device_rate.store(config.sample_rate, Ordering::Relaxed);
    *s.device_name.lock().unwrap() = dev.to_string();
    let s2 = s.clone();
    let s3 = s.clone();
    let err_cb = move |e: cpal::Error| {
        *s3.device_error.lock().unwrap() = Some(e.to_string());
    };
    let mut scratch: Vec<f32> = Vec::new();
    let stream = match fmt {
        cpal::SampleFormat::F32 => dev.build_output_stream::<f32, _, _>(
            config,
            move |data: &mut [f32], _| fill(&s2, data, channels, &mut scratch, |x| x),
            err_cb,
            None,
        )?,
        cpal::SampleFormat::I16 => dev.build_output_stream::<i16, _, _>(
            config,
            move |data: &mut [i16], _| fill(&s2, data, channels, &mut scratch, |x| (x.clamp(-1.0, 1.0) * 32767.0) as i16),
            err_cb,
            None,
        )?,
        cpal::SampleFormat::I32 => dev.build_output_stream::<i32, _, _>(
            config,
            move |data: &mut [i32], _| fill(&s2, data, channels, &mut scratch, |x| (x.clamp(-1.0, 1.0) as f64 * 2147483647.0) as i32),
            err_cb,
            None,
        )?,
        other => anyhow::bail!("Unsupported output sample format {other:?}"),
    };
    stream.play()?;
    Ok(stream)
}

/// Real-time callback body.
fn fill<T: Copy + Default>(s: &Shared, data: &mut [T], channels: usize, scratch: &mut Vec<f32>, conv: impl Fn(f32) -> T) {
    let frames = data.len() / channels.max(1);
    scratch.clear();
    scratch.resize(frames * 2, 0.0);
    let paused = s.paused.load(Ordering::Relaxed);
    let mut got = 0;
    if !paused {
        if let Ok(mut r) = s.ring.try_lock() {
            let n = (frames * 2).min(r.len());
            for (i, v) in r.drain(..n).enumerate() {
                scratch[i] = v;
            }
            got = n / 2;
        }
        s.consumed.fetch_add(got as u64, Ordering::Relaxed);
    }
    // visualizer tap (pre-volume, mono)
    if s.vis_enabled.load(Ordering::Relaxed) && got > 0 {
        if let Ok(mut v) = s.vis.try_lock() {
            for f in scratch[..got * 2].chunks_exact(2) {
                v.push_back((f[0] + f[1]) * 0.5);
            }
            while v.len() > VIS_LEN {
                v.pop_front();
            }
        }
    }
    let vol = if s.muted.load(Ordering::Relaxed) { 0.0 } else { f32_load(&s.volume) };
    let vol = vol * vol; // perceptual curve
    let target = f32_load(&s.fade_target);
    let fade_ms = s.fade_ms.load(Ordering::Relaxed);
    let rate = s.device_rate.load(Ordering::Relaxed).max(1);
    let step = if fade_ms == 0 { 1.0 } else { 1000.0 / (fade_ms as f32 * rate as f32) };
    let mut g = s.current_gain.try_lock().map(|g| *g).unwrap_or(target);
    for f in 0..frames {
        if g < target {
            g = (g + step).min(target);
        } else if g > target {
            g = (g - step).max(target);
        }
        let l = scratch[f * 2] * vol * g;
        let r = scratch[f * 2 + 1] * vol * g;
        let base = f * channels;
        match channels {
            1 => data[base] = conv((l + r) * 0.5),
            _ => {
                data[base] = conv(l);
                data[base + 1] = conv(r);
                for c in 2..channels {
                    data[base + c] = T::default();
                }
            }
        }
    }
    if target == 0.0 && g <= 0.0 && !paused {
        s.paused.store(true, Ordering::Relaxed);
    }
    if let Ok(mut cg) = s.current_gain.try_lock() {
        *cg = g;
    }
}
