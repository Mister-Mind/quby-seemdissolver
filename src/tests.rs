use super::*;
#[test]
fn asymmetric_source_round_trip_and_export() {
    let dir = std::env::temp_dir().join(format!("quby-seemdissolver-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let raw = RgbImage::from_fn(170, 97, |x, y| {
        image::Rgb([x as u8, (y * 2) as u8, ((x + y) / 2) as u8])
    });
    let path = dir.join("input.png");
    raw.save(&path).unwrap();
    let input = load_source(&path).unwrap();
    let settings = Settings::default();
    let maps = material::build(&input, &settings);
    let (w, h) = maps[0].dimensions();
    assert_eq!((w, h), (139, 80));
    for y in 0..h {
        for c in 0..3 {
            assert!(maps[0][(0, y)][c].abs_diff(maps[0][(w - 1, y)][c]) <= 3);
        }
    }
    for x in 0..w {
        for c in 0..3 {
            assert!(maps[0][(x, 0)][c].abs_diff(maps[0][(x, h - 1)][c]) <= 3);
        }
    }
    let p = export(&dir, "Test", &maps, &settings).unwrap();
    assert_eq!(std::fs::read_dir(&p).unwrap().count(), 7);
    for (idx, label) in FILES.iter().enumerate() {
        let label = if idx == 1 { "Normal_OpenGL" } else { label };
        let loaded = image::open(p.join(format!("Test_{label}.png")))
            .unwrap()
            .to_rgb8();
        assert_eq!(loaded, maps[idx]);
    }
    for i in 0..(w * h) as usize {
        let a = i * 3;
        assert_eq!(
            &maps[6].as_raw()[a..a + 3],
            &[
                maps[4].as_raw()[a],
                maps[2].as_raw()[a],
                maps[5].as_raw()[a]
            ]
        );
    }
    let second = export(&dir, "Test", &maps, &settings).unwrap();
    assert_ne!(p, second);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn convention_and_flat_surface() {
    let input = RgbImage::from_pixel(32, 32, image::Rgb([130; 3]));
    let s = Settings::default();
    let flat = material::build(&input, &s);
    assert!(flat[1].pixels().all(|p| p.0 == [128, 128, 255]));
    assert!(flat[4].pixels().all(|p| p.0 == [255; 3]));
    let input = RgbImage::from_fn(100, 100, |x, y| image::Rgb([((x + y) / 2) as u8; 3]));
    let gl = material::build(&input, &s);
    let dx = material::build(&input, &Settings { dx: true, ..s });
    for (a, b) in gl[1].pixels().zip(dx[1].pixels()) {
        assert_eq!(a[0], b[0]);
        assert_eq!(a[2], b[2]);
        assert!((a[1] as i16 + b[1] as i16 - 255).abs() <= 1);
    }
    assert_ne!(
        preview::sphere(&gl, 0.2, false),
        preview::sphere(&gl, 2.8, false)
    );
}

#[test]
fn seam_modes_and_controls() {
    use material::Method;
    let input = RgbImage::from_fn(93, 81, |x, y| {
        image::Rgb([((x * y) % 251) as u8, (y * 3) as u8, (x * 2) as u8])
    });
    let tile = |method, position, feather| {
        material::build(
            &input,
            &Settings {
                method,
                position,
                feather,
                overlap: 0.30,
                overlap_y: 0.10,
                ..Settings::default()
            },
        )
        .remove(0)
    };
    let adaptive = tile(Method::Adaptive, 0.5, 1.);
    let straight = tile(Method::Straight, 0.3, 1.);
    let blend = tile(Method::Blend, 0.5, 1.);
    assert_ne!(adaptive, straight);
    assert_ne!(adaptive, blend);
    assert_ne!(straight, blend);
    assert_ne!(straight, tile(Method::Straight, 0.7, 1.));
    assert_ne!(adaptive, tile(Method::Adaptive, 0.5, 8.));
    for im in [adaptive, straight, blend] {
        let (w, h) = im.dimensions();
        assert_eq!((w, h), (65, 73));
        for x in 0..w {
            assert_eq!(im[(x, 0)], im[(x, h - 1)]);
        }
        for y in 0..h {
            assert_eq!(im[(0, y)], im[(w - 1, y)]);
        }
    }
}

#[test]
fn preview_zoom_keeps_anchor_and_clamps_pan() {
    let mut camera = Camera::default();
    let anchor = Vec2::new(100., -40.);
    camera.zoom(120., anchor);
    assert!(camera.scale > 1.);
    assert!((camera.offset + anchor * camera.scale - anchor).length() < 0.001);
    camera.zoom(9999., anchor);
    assert_eq!(camera.scale, 8.);
    camera.zoom(-9999., anchor);
    assert_eq!(camera.scale, 0.5);
    camera.constrain(Vec2::splat(400.), Vec2::splat(400.));
    assert_eq!(camera.offset, Vec2::ZERO);
}
