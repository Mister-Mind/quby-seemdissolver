use image::{Rgb, RgbImage};
pub const LABELS: [&str; 7] = [
    "Base color",
    "Normal",
    "Roughness",
    "Height",
    "Occlusion",
    "Metallic",
    "ORM",
];
pub const FILES: [&str; 7] = [
    "BaseColor",
    "Normal",
    "Roughness",
    "Height",
    "AO",
    "Metallic",
    "ORM",
];
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Method {
    Adaptive,
    Straight,
    Blend,
}
impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Self::Adaptive => "Adaptive cut",
            Self::Straight => "Straight cut",
            Self::Blend => "Soft blend",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Self::Adaptive => "Follows matching detail through the overlap.",
            Self::Straight => "Place a straight join inside each overlap.",
            Self::Blend => "Crossfades the full overlap for gradual transitions.",
        }
    }
}
#[derive(Clone, PartialEq)]
pub struct Settings {
    pub overlap: f32,
    pub overlap_y: f32,
    pub method: Method,
    pub feather: f32,
    pub position: f32,
    pub relief: f32,
    pub rough: f32,
    pub metal: f32,
    pub dx: bool,
    pub invert: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            overlap: 0.18,
            overlap_y: 0.18,
            method: Method::Adaptive,
            feather: 1.,
            position: 0.5,
            relief: 4.,
            rough: 0.68,
            metal: 0.,
            dx: false,
            invert: false,
        }
    }
}
fn mix(a: Rgb<u8>, b: Rgb<u8>, t: f32) -> Rgb<u8> {
    Rgb(std::array::from_fn(|i| {
        (a[i] as f32 * (1. - t) + b[i] as f32 * t).round() as u8
    }))
}
// Find a low-error path through overlapping opposite edges. Rotate the join into the interior.
fn stitch(src: &RgbImage, s: &Settings, vertical: bool) -> RgbImage {
    let (w, h) = if vertical {
        (src.height(), src.width())
    } else {
        src.dimensions()
    };
    let overlap = if vertical { s.overlap_y } else { s.overlap };
    let band = ((w as f32 * overlap).round() as u32).clamp(4, w / 2);
    let period = w - band;
    let get = |x, y| {
        if vertical {
            *src.get_pixel(y, x)
        } else {
            *src.get_pixel(x, y)
        }
    };
    let n = band as usize;
    let center = (s.position * (n - 1) as f32).round() as usize;
    let mut path = vec![center; h as usize];
    if s.method == Method::Adaptive {
        let mut prev = vec![0f32; n];
        let mut next = prev.clone();
        if vertical {
            prev.fill(f32::INFINITY);
            prev[n / 2] = 0.;
        }
        let mut parent = vec![0i8; n * h as usize];
        for y in 0..h {
            for k in 0..n {
                let a = get(k as u32, y);
                let b = get(period + k as u32, y);
                let lo = k.saturating_sub(1);
                let hi = (k + 1).min(n - 1);
                let p = (lo..=hi)
                    .min_by(|&i, &j| prev[i].total_cmp(&prev[j]))
                    .unwrap();
                let cost = (0..3)
                    .map(|i| (a[i] as f32 - b[i] as f32).powi(2))
                    .sum::<f32>();
                next[k] = prev[p] + cost + if k == 0 || k + 1 == n { 250000. } else { 0. };
                parent[y as usize * n + k] = (p as isize - k as isize) as i8;
            }
            if vertical && y == 0 {
                let start = next[n / 2];
                next.fill(f32::INFINITY);
                next[n / 2] = start;
            }
            std::mem::swap(&mut prev, &mut next);
        }

        let mut k = (0..n).min_by(|&i, &j| prev[i].total_cmp(&prev[j])).unwrap();
        if vertical {
            k = n / 2;
        }
        for y in (0..h as usize).rev() {
            path[y] = k;
            k = (k as isize + parent[y * n + k] as isize) as usize;
        }
    }
    let dim = if vertical { (h, period) } else { (period, h) };
    RgbImage::from_fn(dim.0, dim.1, |x, y| {
        let (u, v) = if vertical { (y, x) } else { (x, y) };
        let u = (u + period / 2) % period;
        if u >= band {
            return get(u, v);
        }
        let t = if s.method == Method::Blend {
            let x = u as f32 / (band - 1) as f32;
            x * x * (3. - 2. * x)
        } else {
            let center = path[v as usize] as f32;
            let radius = s.feather.min(center).min((band - 1) as f32 - center);
            if radius < 0.01 {
                if u as f32 >= center {
                    1.
                } else {
                    0.
                }
            } else {
                ((u as f32 - center) / (2. * radius) + 0.5).clamp(0., 1.)
            }
        };
        mix(get(period + u, v), get(u, v), t)
    })
}
fn gray(v: f32) -> Rgb<u8> {
    Rgb([(v.clamp(0., 1.) * 255.).round() as u8; 3])
}
pub fn build(src: &RgbImage, s: &Settings) -> Vec<RgbImage> {
    let mut base = stitch(&stitch(src, s, false), s, true);
    let (w, h) = base.dimensions();
    // Relax the last few samples where the two independent cuts intersect.
    for vertical in [false, true] {
        let (length, rows) = if vertical { (h, w) } else { (w, h) };
        let width = (length / 16).clamp(1, 4);
        for row in 0..rows {
            for offset in 0..width {
                let point = |u| if vertical { (row, u) } else { (u, row) };
                let a = point(offset);
                let b = point(length - 1 - offset);
                let left = base[a];
                let right = base[b];
                let mid = mix(left, right, 0.5);
                let weight = (1. - offset as f32 / width as f32).powi(2);
                base[a] = mix(left, mid, weight);
                base[b] = mix(right, mid, weight);
            }
        }
    }
    let mut maps = vec![base];
    maps.extend((0..6).map(|_| RgbImage::new(w, h)));
    let z: Vec<f32> = maps[0]
        .pixels()
        .map(|p| {
            let v = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.;
            if s.invert {
                1. - v
            } else {
                v
            }
        })
        .collect();
    let sample = |x: i32, y: i32| {
        z[(y.rem_euclid(h as i32) as u32 * w + x.rem_euclid(w as i32) as u32) as usize]
    };
    for y in 0..h {
        for x in 0..w {
            let (i, j) = (x as i32, y as i32);
            let v = sample(i, j);
            let dx = (sample(i - 1, j) - sample(i + 1, j)) * s.relief;
            let dy = (sample(i, j + 1) - sample(i, j - 1)) * s.relief * if s.dx { -1. } else { 1. };
            let len = (dx * dx + dy * dy + 1.).sqrt();
            let normal = Rgb([dx, dy, 1.].map(|c| ((c / len * 0.5 + 0.5) * 255.).round() as u8));
            let mut occ = 0.;
            for r in [2, 5, 11] {
                for (a, b) in [(r, 0), (-r, 0), (0, r), (0, -r)] {
                    occ += (sample(i + a, j + b) - v).max(0.);
                }
            }
            let ao = gray(1. - occ * s.relief / 18.);
            let rough = gray(s.rough + (dx.abs() + dy.abs()).min(1.) * 0.18 + (0.5 - v) * 0.12);
            let metal = gray(s.metal);
            for (k, p) in [
                normal,
                rough,
                gray(v),
                ao,
                metal,
                Rgb([ao[0], rough[0], metal[0]]),
            ]
            .into_iter()
            .enumerate()
            {
                maps[k + 1].put_pixel(x, y, p);
            }
        }
    }
    maps
}
pub fn demo() -> RgbImage {
    fn noise(x: f32, y: f32) -> f32 {
        let hash = |a: i32, b: i32| {
            let mut k = (a as u32).wrapping_mul(374761393) ^ (b as u32).wrapping_mul(668265263);
            k = (k ^ (k >> 13)).wrapping_mul(1274126177);
            (k ^ (k >> 16)) as f32 / u32::MAX as f32
        };
        let (a, b) = (x.floor() as i32, y.floor() as i32);
        let smooth = |t: f32| t * t * (3. - 2. * t);
        let (u, v) = (smooth(x - x.floor()), smooth(y - y.floor()));
        (hash(a, b) * (1. - u) + hash(a + 1, b) * u) * (1. - v)
            + (hash(a, b + 1) * (1. - u) + hash(a + 1, b + 1) * u) * v
    }
    RgbImage::from_fn(512, 512, |x, y| {
        let (x, y) = (x as f32, y as f32);
        let coarse = noise(x / 96., y / 96.);
        let fine = noise(x / 12., y / 12.);
        let vein = (y / 20. + x / 54. + coarse * 10. + fine * 0.6)
            .sin()
            .abs()
            .powf(0.4);
        let t = 0.10 + 0.44 * vein + 0.21 * coarse + 0.17 * fine + 0.10 * noise(x, y);
        mix(Rgb([64, 38, 31]), Rgb([237, 187, 145]), t)
    })
}
