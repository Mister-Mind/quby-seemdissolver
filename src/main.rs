#![windows_subsystem = "windows"]
mod material;
mod preview;
use eframe::egui::{self, Color32, RichText, TextureHandle, Vec2};
use image::{ImageReader, RgbImage};
use material::{Settings, FILES, LABELS};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    thread,
};

const ACCENT: Color32 = Color32::from_rgb(157, 210, 210);
const MUTED: Color32 = Color32::from_rgb(166, 171, 186);
struct Surface {
    src: Arc<RgbImage>,
    maps: Arc<Vec<RgbImage>>,
    name: String,
    settings: Settings,
}
enum Job {
    Surface(Surface),
    Saved(PathBuf),
}
#[derive(Clone, Copy)]
struct Camera {
    scale: f32,
    offset: Vec2,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            scale: 1.,
            offset: Vec2::ZERO,
        }
    }
}
impl Camera {
    fn zoom(&mut self, wheel: f32, anchor: Vec2) {
        let next = (self.scale * (wheel * 0.004).exp()).clamp(0.5, 8.);
        self.offset = anchor + (self.offset - anchor) * (next / self.scale);
        self.scale = next;
    }
    fn constrain(&mut self, content: Vec2, viewport: Vec2) {
        let limit = ((content * self.scale - viewport) * 0.5).max(Vec2::ZERO);
        self.offset = self.offset.clamp(-limit, limit);
    }
}
struct Studio {
    cameras: [Camera; 3],
    chips: Vec<TextureHandle>,
    logo: TextureHandle,
    grid: bool,
    surface: Surface,
    controls: Settings,
    layer: usize,
    view: usize,
    angle: f32,
    texture: Option<TextureHandle>,
    thumb: Option<TextureHandle>,
    worker: Option<mpsc::Receiver<Result<Job, String>>>,
    status: String,
    observed_controls: Settings,
    regen_at: Option<std::time::Instant>,
}
fn load_source(path: &Path) -> Result<RgbImage, String> {
    let reader = ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
    if w.min(h) < 16 || u64::from(w) * u64::from(h) > 64_000_000 {
        return Err(
            "Choose an image at least 16 pixels on each side and under 64 megapixels.".into(),
        );
    }
    let img = image::open(path).map_err(|e| e.to_string())?;
    let img = if w.max(h) > 4096 {
        img.resize(4096, 4096, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    if img.width().min(img.height()) < 16 {
        return Err("Image is too narrow after scaling.".into());
    }
    Ok(img.to_rgb8())
}
fn export(folder: &Path, name: &str, maps: &[RgbImage], s: &Settings) -> Result<PathBuf, String> {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(70)
        .collect();
    let mut number = 1;
    let dest = loop {
        let p = folder.join(format!("{clean}_PBR_{number:02}"));
        match std::fs::create_dir(&p) {
            Ok(()) => break p,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => number += 1,
            Err(e) => return Err(e.to_string()),
        }
    };
    let write = || -> Result<(), String> {
        for (map, label) in maps.iter().zip(FILES) {
            let label = if label == "Normal" {
                if s.dx {
                    "Normal_DirectX"
                } else {
                    "Normal_OpenGL"
                }
            } else {
                label
            };
            map.save(dest.join(format!("{clean}_{label}.png")))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    };
    if let Err(e) = write() {
        return Err(format!(
            "Export stopped. Partial files in {}. {e}",
            dest.display()
        ));
    }
    Ok(dest)
}
fn texture(ctx: &egui::Context, id: &str, img: &RgbImage) -> TextureHandle {
    ctx.load_texture(
        id,
        egui::ColorImage::from_rgb([img.width() as usize, img.height() as usize], img.as_raw()),
        egui::TextureOptions {
            wrap_mode: egui::TextureWrapMode::Repeat,
            ..egui::TextureOptions::LINEAR
        },
    )
}
fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .font(egui::FontId::new(
                13.,
                egui::FontFamily::Name("Display".into()),
            ))
            .color(MUTED),
    );
}
fn artwork() -> image::RgbaImage {
    image::load_from_memory(include_bytes!("../assets/mark.png"))
        .expect("embedded icon")
        .into_rgba8()
}
impl Studio {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        for (name, bytes) in [(
            "Barlow display",
            include_bytes!("../assets/fonts/BarlowSemiCondensed-SemiBold.ttf").as_slice(),
        )] {
            fonts
                .font_data
                .insert(name.into(), egui::FontData::from_static(bytes).into());
        }
        let display = egui::FontFamily::Name("Display".into());
        let mut title_font = fonts.families[&egui::FontFamily::Proportional].clone();
        title_font.insert(0, "Barlow display".into());
        fonts.families.insert(display.clone(), title_font);
        cc.egui_ctx.set_fonts(fonts);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(10., 8.);
        style.spacing.button_padding = Vec2::new(12., 7.);
        style.visuals.panel_fill = Color32::from_rgb(24, 25, 31);
        style.visuals.selection.bg_fill = Color32::from_rgb(58, 55, 77);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(57, 83, 90);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(50, 52, 65);
        style.visuals.widgets.hovered.fg_stroke.color = Color32::from_rgb(219, 211, 235);
        style.visuals.widgets.active.fg_stroke.color = ACCENT;
        style.visuals.hyperlink_color = ACCENT;
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(26., display.clone()),
        );
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::new(16., display));
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(15.));
        cc.egui_ctx.set_style(style);
        let mark = artwork();
        let logo = cc.egui_ctx.load_texture(
            "quby mark",
            egui::ColorImage::from_rgba_unmultiplied(
                [mark.width() as usize, mark.height() as usize],
                mark.as_raw(),
            ),
            egui::TextureOptions::LINEAR,
        );
        let src = Arc::new(material::demo());
        let settings = Settings::default();
        let maps = Arc::new(material::build(&src, &settings));
        Self {
            logo,
            cameras: [Camera::default(); 3],
            chips: Vec::new(),
            grid: false,
            surface: Surface {
                src,
                maps,
                name: "Strata sandstone".into(),
                settings: settings.clone(),
            },
            controls: settings.clone(),
            layer: 0,
            view: 2,
            angle: 0.92,
            texture: None,
            thumb: None,
            worker: None,
            status: "Demo material loaded".into(),
            observed_controls: settings.clone(),
            regen_at: None,
        }
    }
    fn run(
        &mut self,
        ctx: &egui::Context,
        msg: &str,
        f: impl FnOnce() -> Result<Job, String> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.status = msg.into();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
    }
    fn generate(&mut self, ctx: &egui::Context, path: Option<PathBuf>) {
        let s = self.controls.clone();
        let src = self.surface.src.clone();
        let name = self.surface.name.clone();
        self.run(ctx, "Building material…", move || {
            let (src, name) = if let Some(p) = path {
                (
                    Arc::new(load_source(&p)?),
                    p.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                )
            } else {
                (src, name)
            };
            let maps = Arc::new(material::build(&src, &s));
            Ok(Job::Surface(Surface {
                src,
                maps,
                name,
                settings: s,
            }))
        })
    }
    fn refresh(&mut self, ctx: &egui::Context) {
        if self.texture.is_some() {
            return;
        }
        let img = if self.view == 2 {
            preview::sphere(&self.surface.maps, self.angle, self.surface.settings.dx)
        } else {
            let img = &self.surface.maps[self.layer];
            if img.width().max(img.height()) > 1536 {
                image::DynamicImage::ImageRgb8(img.clone())
                    .thumbnail(1536, 1536)
                    .to_rgb8()
            } else {
                img.clone()
            }
        };
        self.texture = Some(texture(ctx, "inspection", &img));
    }
}
impl eframe::App for Studio {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(rx) = &self.worker {
            let result = match rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Processing stopped unexpectedly.".into()))
                }
                _ => None,
            };
            if let Some(r) = result {
                self.worker = None;
                match r {
                    Ok(Job::Surface(s)) => {
                        self.surface = s;
                        self.texture = None;
                        self.thumb = None;
                        self.chips.clear();
                        self.status = "Material ready".into()
                    }
                    Ok(Job::Saved(p)) => self.status = format!("Exported to {}", p.display()),
                    Err(e) => self.status = e,
                }
            }
        }
        if self.controls != self.observed_controls {
            self.observed_controls = self.controls.clone();
            self.regen_at = Some(std::time::Instant::now() + std::time::Duration::from_millis(180));
        }
        let busy = self.worker.is_some();
        let dirty = self.controls != self.surface.settings;
        if dirty
            && !busy
            && self
                .regen_at
                .is_some_and(|at| std::time::Instant::now() >= at)
        {
            self.regen_at = None;
            self.generate(ctx, None);
        }
        if self.regen_at.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
        if !busy {
            if let Some(path) =
                ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone()))
            {
                self.generate(ctx, Some(path))
            }
        }
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(7.);
            ui.horizontal(|ui| {
                ui.add(egui::Image::new(&self.logo).fit_to_exact_size(Vec2::splat(44.)));
                ui.heading("quby seemdissolver");
                ui.separator();
                ui.label(RichText::new("Material studio").color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !busy && !dirty,
                            egui::Button::new(
                                RichText::new("Export PBR set")
                                    .strong()
                                    .color(Color32::BLACK),
                            )
                            .fill(ACCENT),
                        )
                        .on_hover_text("Write seven PNG maps into a new folder")
                        .clicked()
                    {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Choose parent folder for material")
                            .pick_folder()
                        {
                            let maps = self.surface.maps.clone();
                            let name = self.surface.name.clone();
                            let s = self.surface.settings.clone();
                            self.run(ctx, "Writing PNG maps…", move || {
                                export(&p, &name, &maps, &s).map(Job::Saved)
                            });
                        }
                    }
                });
            });
            ui.add_space(4.);
            let (strip, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 2.), egui::Sense::hover());
            let mut mesh = egui::Mesh::default();
            for (fraction, color) in [
                (0., ACCENT),
                (0.20, Color32::from_rgb(143, 132, 166)),
                (0.48, Color32::from_rgb(48, 52, 65)),
                (1., Color32::from_rgb(31, 33, 39)),
            ] {
                let x = strip.left() + strip.width() * fraction;
                mesh.colored_vertex(egui::pos2(x, strip.top()), color);
                mesh.colored_vertex(egui::pos2(x, strip.bottom()), color);
            }
            for i in [0, 2, 4] {
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i + 1, i + 3, i + 2);
            }
            ui.painter().add(egui::Shape::mesh(mesh));
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if busy {
                    ui.spinner();
                }
                ui.label(
                    RichText::new(if dirty && !busy {
                        "Updating material..."
                    } else {
                        &self.status
                    })
                    .size(12.),
                );
            });
        });
        egui::SidePanel::left("source")
            .exact_width(254.)
            .resizable(false)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(12.);
                    caption(ui, "SOURCE IMAGE");
                    ui.heading(&self.surface.name);
                    if self.thumb.is_none() {
                        let img = image::DynamicImage::ImageRgb8((*self.surface.src).clone())
                            .thumbnail(360, 180)
                            .to_rgb8();
                        self.thumb = Some(texture(ctx, "source", &img));
                    }
                    let thumb = self.thumb.as_ref().unwrap();
                    ui.add(egui::Image::new(thumb).max_size(Vec2::new(210., 100.)));
                    ui.small(format!(
                        "{} × {} px",
                        self.surface.src.width(),
                        self.surface.src.height()
                    ));
                    if ui
                        .add_enabled(!busy, egui::Button::new("Import image…"))
                        .on_hover_text("Or drop an image onto this window")
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter(
                                "Texture image",
                                &["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff"],
                            )
                            .pick_file()
                        {
                            self.generate(ctx, Some(path));
                        }
                    }
                    ui.add_space(6.);
                    ui.separator();
                    caption(ui, "SEAM CONTROLS");
                    ui.add_enabled_ui(true, |ui| {
                        use material::Method;
                        egui::ComboBox::from_id_salt("seam_method")
                            .selected_text(self.controls.method.label())
                            .width(190.)
                            .show_ui(ui, |ui| {
                                for method in [Method::Adaptive, Method::Straight, Method::Blend] {
                                    ui.selectable_value(
                                        &mut self.controls.method,
                                        method,
                                        method.label(),
                                    );
                                }
                            });
                        ui.small(self.controls.method.hint());
                        for (label, value) in [
                            ("Horizontal overlap", &mut self.controls.overlap),
                            ("Vertical overlap", &mut self.controls.overlap_y),
                        ] {
                            ui.label(label);
                            ui.add(
                                egui::Slider::new(value, 0.08..=0.35)
                                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.))
                                    .custom_parser(|s| {
                                        s.trim_end_matches('%')
                                            .parse::<f64>()
                                            .ok()
                                            .map(|v| v / 100.)
                                    }),
                            );
                        }
                        if self.controls.method != Method::Blend {
                            ui.label("Feather radius");
                            ui.add(
                                egui::Slider::new(&mut self.controls.feather, 0.0..=24.)
                                    .suffix(" px"),
                            );
                        }
                        if self.controls.method == Method::Straight {
                            ui.label("Cut position in overlap");
                            ui.add(
                                egui::Slider::new(&mut self.controls.position, 0.1..=0.9)
                                    .fixed_decimals(2),
                            );
                        }
                        ui.separator();
                        caption(ui, "PBR CONTROLS");
                        for (label, val, range) in [
                            ("Relief strength", &mut self.controls.relief, 0.0..=10.0),
                            ("Roughness", &mut self.controls.rough, 0.0..=1.0),
                            ("Metallic", &mut self.controls.metal, 0.0..=1.0),
                        ] {
                            ui.label(label);
                            ui.add(egui::Slider::new(val, range).fixed_decimals(2));
                        }
                        ui.checkbox(&mut self.controls.invert, "Invert height");
                        ui.checkbox(&mut self.controls.dx, "DirectX normals (−Y)");
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&ctx.style())
                    .fill(Color32::from_rgb(23, 25, 28))
                    .inner_margin(16),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (i, name) in ["Single tile", "3 × 3 repeat", "Lit sphere"]
                        .iter()
                        .enumerate()
                    {
                        if ui.selectable_label(self.view == i, *name).clicked() {
                            self.view = i;
                            self.texture = None;
                        }
                    }
                    ui.separator();
                    ui.monospace(format!("{:.0}%", self.cameras[self.view].scale * 100.));
                    if ui
                        .small_button("Fit")
                        .on_hover_text("Reset zoom and pan")
                        .clicked()
                    {
                        self.cameras[self.view] = Camera::default();
                    }
                });
                if self.view == 1 {
                    ui.checkbox(&mut self.grid, "Show tile boundaries");
                }
                if self.view == 2 {
                    ui.horizontal(|ui| {
                        ui.small("Light angle");
                        if ui
                            .add(egui::Slider::new(&mut self.angle, 0.05..=3.09).show_value(false))
                            .changed()
                        {
                            self.texture = None;
                        }
                    });
                } else {
                    ui.small(format!(
                        "{}   /   {} × {} px",
                        LABELS[self.layer],
                        self.surface.maps[0].width(),
                        self.surface.maps[0].height()
                    ));
                }
                self.refresh(ctx);
                let tex = self.texture.as_ref().unwrap();
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), (ui.available_height() - 94.).max(60.)),
                    egui::Sense::click_and_drag(),
                );
                let ratio = tex.size_vec2();
                let size = ratio * ((rect.width() / ratio.x).min(rect.height() / ratio.y));
                let camera = &mut self.cameras[self.view];
                if response.hovered() {
                    if let Some(pointer) = response.hover_pos() {
                        camera.zoom(
                            ui.input(|i| i.smooth_scroll_delta.y),
                            pointer - rect.center(),
                        );
                    }
                }
                if response.dragged() {
                    camera.offset += ui.input(|i| i.pointer.delta());
                }
                if response.double_clicked() {
                    *camera = Camera::default();
                }
                camera.constrain(size, rect.size());
                response.on_hover_text("Wheel to zoom · drag to pan · double-click to fit");
                let painter = ui.painter_at(rect);
                let target = egui::Rect::from_center_size(
                    rect.center() + camera.offset,
                    size * camera.scale,
                );
                let reps = if self.view == 1 { 3. } else { 1. };
                painter.image(
                    tex.id(),
                    target,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(reps, reps)),
                    Color32::WHITE,
                );
                if self.view == 1 && self.grid {
                    for i in [1., 2.] {
                        let x = target.left() + target.width() * i / 3.;
                        let y = target.top() + target.height() * i / 3.;
                        for (a, b) in [
                            (egui::pos2(x, target.top()), egui::pos2(x, target.bottom())),
                            (egui::pos2(target.left(), y), egui::pos2(target.right(), y)),
                        ] {
                            painter.line_segment([a, b], (3., Color32::from_black_alpha(150)));
                            painter.line_segment([a, b], (1., ACCENT));
                        }
                    }
                }
                ui.add_space(6.);
                ui.separator();
                caption(ui, "TEXTURE MAPS");
                if self.chips.is_empty() {
                    self.chips = self
                        .surface
                        .maps
                        .iter()
                        .enumerate()
                        .map(|(i, m)| {
                            let (w, h) = m.dimensions();
                            let d = w.min(h);
                            let crop = image::imageops::crop_imm(m, (w - d) / 2, (h - d) / 2, d, d);
                            let chip = image::imageops::thumbnail(&*crop, 32, 32);
                            texture(ctx, &format!("map-chip-{i}"), &chip)
                        })
                        .collect();
                }
                ui.horizontal_wrapped(|ui| {
                    for (i, name) in LABELS.iter().enumerate() {
                        if ui
                            .add(
                                egui::Button::image_and_text(
                                    egui::Image::new(&self.chips[i])
                                        .fit_to_exact_size(Vec2::splat(19.)),
                                    *name,
                                )
                                .selected(self.layer == i && self.view != 2),
                            )
                            .clicked()
                        {
                            self.layer = i;
                            if self.view == 2 {
                                self.view = 0
                            }
                            self.texture = None;
                        }
                    }
                });
            });
    }
}
fn main() -> eframe::Result {
    eframe::run_native(
        "quby seemdissolver",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_icon({
                    let img = artwork();
                    egui::IconData {
                        width: img.width(),
                        height: img.height(),
                        rgba: img.into_raw(),
                    }
                })
                .with_inner_size([1120., 780.])
                .with_min_inner_size([820., 580.]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(Studio::new(cc)))),
    )
}

#[cfg(test)]
mod tests;
