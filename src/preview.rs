use image::{Rgb, RgbImage};
pub fn sphere(m: &[RgbImage], angle: f32, directx: bool, orbit: [f32; 2]) -> RgbImage {
    let size = 448;
    let light = [angle.cos() * 0.8, 0.6, angle.sin() * 0.8];
    let (w, h) = m[0].dimensions();
    RgbImage::from_fn(size, size, |x, y| {
        let px = (x as f32 + 0.5 - size as f32 / 2.) / (size as f32 * 0.425);
        let py = (size as f32 / 2. - y as f32 - 0.5) / (size as f32 * 0.425);
        let rr = px * px + py * py;
        if rr >= 1. {
            let shadow = (-px * px * 3. - (py + 0.98).powi(2) * 95.).exp();
            return Rgb([23., 25., 28.].map(|v| (v - 12. * shadow) as u8));
        }
        let pz = (1. - rr).sqrt();
        let phi = px.atan2(pz) + orbit[0];
        let latitude = (py.asin() + orbit[1]).clamp(-1.55, 1.55);
        let u = ((0.5 + phi / std::f32::consts::TAU) * 3.).fract();
        let v = ((0.5 - latitude / std::f32::consts::PI) * 2.).fract();
        let (i, j) = ((u * w as f32) as u32 % w, (v * h as f32) as u32 % h);
        let [nx, ny, nz] = m[1].get_pixel(i, j).0.map(|c| c as f32 / 127.5 - 1.);
        let ny = if directx { -ny } else { ny };
        let n = [
            nx * phi.cos() - ny * py * phi.sin() + nz * px,
            ny * (1. - py * py).sqrt() + nz * py,
            -nx * phi.sin() - ny * py * phi.cos() + nz * pz,
        ];
        let dot = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.);
        let rough = m[2].get_pixel(i, j)[0] as f32 / 255.;
        let metal = m[5].get_pixel(i, j)[0] as f32 / 255.;
        let half_len = (2. + 2. * light[2]).sqrt();
        let nh =
            ((n[0] * light[0] + n[1] * light[1] + n[2] * (light[2] + 1.)) / half_len).clamp(0., 1.);
        let spec = nh.powf(3. + (1. - rough).powi(3) * 160.) * (1. - rough * 0.65);
        let ao = m[4].get_pixel(i, j)[0] as f32 / 255.;
        Rgb(m[0].get_pixel(i, j).0.map(|c| {
            let c = (c as f32 / 255.).powf(2.2);
            let lum = c * (0.23 + dot * 0.82) * (1. - metal * 0.72) * ao
                + spec * (0.08 * (1. - metal) + c * metal);
            (lum.clamp(0., 1.).powf(1. / 2.2) * 255.) as u8
        }))
    })
}
