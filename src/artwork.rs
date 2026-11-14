//! Artwork loading and caching with a byte budget.
//!
//! One worker thread downloads (or reads from the disk cache), decodes and
//! downsizes images to `max_px` before they ever reach the GPU; the UI thread
//! uploads finished images as textures and evicts least-recently-used ones when
//! the budget is exceeded. Blurred backgrounds are produced on the worker at a
//! small resolution (they are blurry anyway), which keeps them cheap.

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Thumbnail/cover scaled to max_px.
    Cover,
    /// Background: scaled to fit `px`, optionally blurred with `radius`.
    Background { px: u32, blur: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub src: String,
    pub kind: Kind,
}

struct Req {
    key: Key,
    max_px: u32,
    disk_cache: bool,
}

struct Resp {
    key: Key,
    img: Result<ColorImage, String>,
    dominant: Option<[u8; 3]>,
}

struct Entry {
    tex: TextureHandle,
    bytes: usize,
    last_used: Instant,
    dominant: Option<[u8; 3]>,
}

pub struct Artwork {
    tx: mpsc::Sender<Req>,
    rx: mpsc::Receiver<Resp>,
    entries: HashMap<Key, Entry>,
    pending: HashSet<Key>,
    failed: HashSet<Key>,
    pub budget_bytes: usize,
    pub used_bytes: usize,
    pub max_px: u32,
    pub disk_cache: bool,
}

impl Artwork {
    pub fn new(ctx: egui::Context, budget_mb: u32, max_px: u32) -> Artwork {
        let (tx, wrx) = mpsc::channel::<Req>();
        let (wtx, rx) = mpsc::channel::<Resp>();
        std::thread::Builder::new()
            .name("artwork".into())
            .spawn(move || {
                while let Ok(r) = wrx.recv() {
                    let img = load(&r.key, r.max_px, r.disk_cache);
                    let dominant = img.as_ref().ok().map(dominant_color);
                    if wtx.send(Resp { key: r.key, img, dominant }).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("artwork thread");
        Artwork {
            tx,
            rx,
            entries: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            budget_bytes: budget_mb as usize * 1024 * 1024,
            used_bytes: 0,
            max_px,
            disk_cache: true,
        }
    }

    /// Call once per frame: upload finished images.
    pub fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(r) = self.rx.try_recv() {
            self.pending.remove(&r.key);
            match r.img {
                Ok(img) => {
                    let bytes = img.pixels.len() * 4;
                    let tex = ctx.load_texture(format!("art:{:?}", r.key), img, TextureOptions::LINEAR);
                    self.used_bytes += bytes;
                    self.entries.insert(r.key, Entry { tex, bytes, last_used: Instant::now(), dominant: r.dominant });
                }
                Err(e) => {
                    log::debug!("artwork failed: {e}");
                    self.failed.insert(r.key);
                }
            }
        }
        self.evict_to(self.budget_bytes);
    }

    pub fn get(&mut self, src: &str, kind: Kind) -> Option<TextureHandle> {
        if src.is_empty() {
            return None;
        }
        let key = Key { src: src.to_string(), kind };
        if let Some(e) = self.entries.get_mut(&key) {
            e.last_used = Instant::now();
            return Some(e.tex.clone());
        }
        if !self.pending.contains(&key) && !self.failed.contains(&key) {
            self.pending.insert(key.clone());
            let _ = self.tx.send(Req { key, max_px: self.max_px, disk_cache: self.disk_cache });
        }
        None
    }

    pub fn dominant(&self, src: &str) -> Option<[u8; 3]> {
        self.entries.iter().find(|(k, _)| k.src == src).and_then(|(_, e)| e.dominant)
    }

    /// Request ahead of time without needing the texture yet.
    pub fn prefetch(&mut self, src: &str) {
        let _ = self.get(src, Kind::Cover);
    }

    pub fn evict_to(&mut self, budget: usize) {
        if self.used_bytes <= budget {
            return;
        }
        let mut v: Vec<(Key, Instant)> = self.entries.iter().map(|(k, e)| (k.clone(), e.last_used)).collect();
        v.sort_by_key(|(_, t)| *t);
        let now = Instant::now();
        for (k, t) in v {
            if self.used_bytes <= budget {
                break;
            }
            // never evict something used in the last second (on screen)
            if now.duration_since(t).as_millis() < 1000 && budget > 0 {
                continue;
            }
            if let Some(e) = self.entries.remove(&k) {
                self.used_bytes -= e.bytes;
            }
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.entries.shrink_to_fit();
        self.used_bytes = 0;
        self.failed.clear();
    }

    pub fn clear_kind_backgrounds(&mut self) {
        let keys: Vec<Key> = self.entries.keys().filter(|k| matches!(k.kind, Kind::Background { .. })).cloned().collect();
        for k in keys {
            if let Some(e) = self.entries.remove(&k) {
                self.used_bytes -= e.bytes;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn retry_failed(&mut self) {
        self.failed.clear();
    }
}

pub fn disk_dir() -> std::path::PathBuf {
    crate::util::cache_dir().join("artwork")
}

pub fn disk_usage() -> u64 {
    std::fs::read_dir(disk_dir()).into_iter().flatten().flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()
}

pub fn clear_disk() {
    let _ = std::fs::remove_dir_all(disk_dir());
}

fn cache_name(src: &str, px: u32) -> String {
    use sha2::Digest;
    let h = sha2::Sha256::digest(format!("{src}@{px}").as_bytes());
    h.iter().take(16).map(|b| format!("{b:02x}")).collect::<String>() + ".jpg"
}

fn fetch_bytes(src: &str) -> Result<Vec<u8>, String> {
    if src.starts_with("http://") || src.starts_with("https://") {
        let r = crate::services::http().get(src).send().map_err(|e| e.to_string())?;
        if !r.status().is_success() {
            return Err(format!("HTTP {}", r.status()));
        }
        let b = r.bytes().map_err(|e| e.to_string())?;
        if b.len() > 20 * 1024 * 1024 {
            return Err("image too large".into());
        }
        Ok(b.to_vec())
    } else {
        let meta = std::fs::metadata(src).map_err(|e| e.to_string())?;
        if meta.len() > 64 * 1024 * 1024 {
            return Err("image file too large".into());
        }
        std::fs::read(src).map_err(|e| e.to_string())
    }
}

fn load(key: &Key, max_px: u32, disk_cache: bool) -> Result<ColorImage, String> {
    let (px, blur) = match key.kind {
        Kind::Cover => (max_px, 0),
        Kind::Background { px, blur } => (px, blur),
    };
    let remote = key.src.starts_with("http");
    let cache_path = disk_dir().join(cache_name(&key.src, px));
    let img = if remote && disk_cache && blur == 0 && cache_path.exists() {
        image::open(&cache_path).map_err(|e| e.to_string())?
    } else {
        let bytes = fetch_bytes(&key.src)?;
        let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        let img = if img.width() > px || img.height() > px { img.thumbnail(px, px) } else { img };
        if remote && disk_cache && blur == 0 {
            let _ = crate::util::ensure_dir(&disk_dir());
            let _ = img.to_rgb8().save_with_format(&cache_path, image::ImageFormat::Jpeg);
        }
        img
    };
    let mut rgba = img.to_rgba8();
    if blur > 0 {
        // blur at reduced resolution: cheap and visually equivalent
        let small = image::imageops::thumbnail(&rgba, (rgba.width() / 2).max(1), (rgba.height() / 2).max(1));
        rgba = box_blur(small, (blur / 2).max(1));
    }
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// Three-pass separable box blur ≈ Gaussian.
pub fn box_blur(mut img: image::RgbaImage, radius: u32) -> image::RgbaImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 {
        return img;
    }
    let r = radius.min(64) as usize;
    let mut tmp = vec![[0u8; 4]; w.max(h)];
    for _ in 0..3 {
        for y in 0..h {
            blur_line(&mut img, &mut tmp, r, w, |i| (i as u32, y as u32));
        }
        for x in 0..w {
            blur_line(&mut img, &mut tmp, r, h, |i| (x as u32, i as u32));
        }
    }
    img
}

fn blur_line(img: &mut image::RgbaImage, tmp: &mut [[u8; 4]], r: usize, n: usize, at: impl Fn(usize) -> (u32, u32)) {
    let mut acc = [0u32; 4];
    let px = |img: &image::RgbaImage, i: isize| -> [u8; 4] {
        let i = i.clamp(0, n as isize - 1) as usize;
        let (x, y) = at(i);
        img.get_pixel(x, y).0
    };
    for i in -(r as isize)..=(r as isize) {
        let p = px(img, i);
        for c in 0..4 {
            acc[c] += p[c] as u32;
        }
    }
    let d = (2 * r + 1) as u32;
    for i in 0..n {
        tmp[i] = [0, 1, 2, 3].map(|c| (acc[c] / d) as u8);
        let add = px(img, i as isize + r as isize + 1);
        let sub = px(img, i as isize - r as isize);
        for c in 0..4 {
            acc[c] = acc[c] + add[c] as u32 - sub[c] as u32;
        }
    }
    for (i, t) in tmp.iter().enumerate().take(n) {
        let (x, y) = at(i);
        img.put_pixel(x, y, image::Rgba(*t));
    }
}

/// Average of reasonably saturated, mid-bright pixels.
pub fn dominant_color(img: &ColorImage) -> [u8; 3] {
    let mut sum = [0u64; 3];
    let mut n = 0u64;
    let step = (img.pixels.len() / 1024).max(1);
    for p in img.pixels.iter().step_by(step) {
        let [r, g, b, _] = p.to_array();
        let mx = r.max(g).max(b) as i32;
        let mn = r.min(g).min(b) as i32;
        if mx > 60 && mx - mn > 40 {
            sum[0] += r as u64;
            sum[1] += g as u64;
            sum[2] += b as u64;
            n += 1;
        }
    }
    if n == 0 {
        return [34, 211, 238];
    }
    let c = [sum[0] / n, sum[1] / n, sum[2] / n].map(|v| v as u8);
    // lift brightness so it works as an accent on dark backgrounds
    let mx = c[0].max(c[1]).max(c[2]).max(1) as f32;
    let k = (200.0 / mx).max(1.0);
    c.map(|v| (v as f32 * k).min(255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blur_smooths_edge() {
        let mut img = image::RgbaImage::new(32, 1);
        for x in 16..32 {
            img.put_pixel(x, 0, image::Rgba([255, 255, 255, 255]));
        }
        let b = box_blur(img, 3);
        let v = b.get_pixel(16, 0).0[0];
        assert!(v > 60 && v < 200, "{v}");
        assert_eq!(b.get_pixel(0, 0).0[0], 0);
    }
    #[test]
    fn dominant_of_red() {
        let img = ColorImage::new([8, 8], vec![egui::Color32::from_rgb(200, 20, 20); 64]);
        let d = dominant_color(&img);
        assert!(d[0] > 150 && d[1] < 60);
    }
    #[test]
    fn local_file_load_and_downscale() {
        let p = std::env::temp_dir().join(format!("aqua-art-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(1000, 500, image::Rgba([10, 200, 220, 255])).save(&p).unwrap();
        let k = Key { src: p.to_string_lossy().to_string(), kind: Kind::Cover };
        let img = load(&k, 200, false).unwrap();
        assert_eq!(img.size, [200, 100]);
        let k2 = Key { src: p.to_string_lossy().to_string(), kind: Kind::Background { px: 400, blur: 10 } };
        let img2 = load(&k2, 200, false).unwrap();
        assert_eq!(img2.size, [200, 100]);
        let _ = std::fs::remove_file(p);
    }
}
