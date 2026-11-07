#![windows_subsystem = "windows"]
mod library;
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
fn lang(french: bool, english: &'static str, français: &'static str) -> &'static str {
    if french {
        français
    } else {
        english
    }
}
struct Surface {
    src: Arc<RgbImage>,
    maps: Arc<Vec<RgbImage>>,
    name: String,
    settings: Settings,
}
enum Job {
    Surface(Surface),
    Saved(PathBuf),
    Assets(Vec<library::Asset>),
    Downloaded(PathBuf),
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
    crop_tab: bool,
    crop_rect: egui::Rect,
    crop_ratio: usize,
    crop_texture: Option<TextureHandle>,
    french: bool,
    library_tab: bool,
    provider: library::Provider,
    search_text: String,
    assets: Vec<library::Asset>,
    library_thumbs: std::collections::HashMap<String, TextureHandle>,
    library_previews: Option<mpsc::Receiver<(String, Option<image::RgbaImage>)>>,
    selected_asset: Option<usize>,
    orbit: Vec2,
    mapping: usize,
    show_blender_export: bool,
    resolution: String,
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
fn export_blender(name: &str, maps: &[RgbImage], mapping: usize) -> Result<PathBuf, String> {
    let library = std::env::var("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("Documents")
        .join("Blender Asset Libraries")
        .join("Quby Materials");
    std::fs::create_dir_all(&library).map_err(|e| e.to_string())?;
    let catalog = "bfb8ef08-9dd7-4f5a-bdb6-c3984b32ccfa";
    let catalog_file = library.join("blender_assets.cats.txt");
    if !catalog_file.exists() {
        std::fs::write(&catalog_file,format!("# This is an Asset Catalog Definition file for Blender.\nVERSION 1\n{catalog}:Quby Materials:Quby Materials\n")).map_err(|e|e.to_string())?;
    }
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(70)
        .collect();
    let mut output = library.join(format!("{clean}.blend"));
    let mut suffix = 2;
    while output.exists() {
        output = library.join(format!("{clean}_{suffix}.blend"));
        suffix += 1;
    }
    let root = std::env::temp_dir().join(format!("quby-blender-{}", std::process::id()));
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    for (img, label) in maps.iter().zip([
        "BaseColor",
        "Normal",
        "Roughness",
        "Height",
        "AO",
        "Metallic",
    ]) {
        let p = root.join(format!("{label}.png"));
        img.save(&p).map_err(|e| e.to_string())?;
        files.push(p);
    }
    let script = root.join("make_asset.py");
    std::fs::write(&script, BLENDER_ASSET_SCRIPT).map_err(|e| e.to_string())?;
    let blender = [
        r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe",
        r"C:\Program Files\Blender Foundation\Blender 5.1\blender.exe",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
    .ok_or("Blender 5.1 or newer was not found")?;
    let mut cmd = std::process::Command::new(blender);
    cmd.args(["--background", "--python"])
        .arg(&script)
        .arg("--")
        .arg(&output)
        .arg(name)
        .arg(mapping.to_string())
        .arg(&library)
        .arg(catalog);
    for f in files {
        cmd.arg(f);
    }
    let result = cmd.output().map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr)
            .chars()
            .rev()
            .take(900)
            .collect::<String>()
            .chars()
            .rev()
            .collect());
    }
    Ok(output)
}
const BLENDER_ASSET_SCRIPT: &str = r#"
import bpy,sys
from pathlib import Path
a=sys.argv[sys.argv.index('--')+1:]; out,name,mode,library,catalog,*paths=a
bpy.ops.object.select_all(action='SELECT');bpy.ops.object.delete(use_global=False)
m=bpy.data.materials.new(name);m.diffuse_color=(.32,.34,.36,1);m.use_nodes=True
n=m.node_tree.nodes;l=m.node_tree.links;n.clear();outn=n.new('ShaderNodeOutputMaterial');bs=n.new('ShaderNodeBsdfPrincipled');l.new(bs.outputs['BSDF'],outn.inputs['Surface'])
coords=n.new('ShaderNodeTexCoord');mapping=n.new('ShaderNodeVectorMath');mapping.operation='SCALE';mapping.inputs[3].default_value=1.0
if mode=='0': vector=coords.outputs['UV']
else: vector=coords.outputs['Generated']
for i,(p,color) in enumerate(zip(paths,[True,False,False,False,False,False])):
 tex=n.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(p,check_existing=True);tex.image.pack();tex.interpolation='Smart';tex.projection='BOX' if mode in ('1','2') else 'FLAT';tex.projection_blend=.25 if mode=='2' else 0.;tex.image.colorspace_settings.name='sRGB' if color else 'Non-Color';l.new(vector,tex.inputs['Vector'])
 if i==0:l.new(tex.outputs['Color'],bs.inputs['Base Color'])
 elif i==1:
  normal=n.new('ShaderNodeNormalMap');l.new(tex.outputs['Color'],normal.inputs['Color']);l.new(normal.outputs['Normal'],bs.inputs['Normal'])
 elif i==2:l.new(tex.outputs['Color'],bs.inputs['Roughness'])
 elif i==3:
  bump=n.new('ShaderNodeBump');l.new(tex.outputs['Color'],bump.inputs['Height']);l.new(bump.outputs['Normal'],bs.inputs['Normal'])
 elif i==5:l.new(tex.outputs['Color'],bs.inputs['Metallic'])
m.asset_mark();m.asset_data.description='Quby seemdissolver PBR material';m.asset_data.catalog_id=catalog
root=Path(library).resolve();prefs=bpy.context.preferences.filepaths.asset_libraries
if not any(Path(item.path).resolve()==root for item in prefs): prefs.new(name='Quby Materials',directory=str(root))
bpy.ops.wm.save_userpref();bpy.ops.wm.save_as_mainfile(filepath=str(Path(out)))
"#;
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
    ui.label(RichText::new(text).color(MUTED));
}
fn artwork() -> image::RgbaImage {
    image::load_from_memory(include_bytes!("../assets/mark.png"))
        .expect("embedded icon")
        .into_rgba8()
}
impl Studio {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
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
            status: lang(
                false,
                "Demo material loaded",
                "Matériau de démonstration chargé",
            )
            .into(),
            observed_controls: settings.clone(),
            regen_at: None,
            crop_tab: false,
            crop_rect: egui::Rect::from_min_max(egui::pos2(0.08, 0.08), egui::pos2(0.92, 0.92)),
            crop_ratio: 0,
            crop_texture: None,
            french: false,
            library_tab: false,
            provider: library::Provider::PolyHaven,
            search_text: String::new(),
            assets: Vec::new(),
            library_thumbs: std::collections::HashMap::new(),
            library_previews: None,
            selected_asset: None,
            orbit: Vec2::ZERO,
            mapping: 0,
            show_blender_export: false,
            resolution: "2k".into(),
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
    fn generate_crop(&mut self, ctx: &egui::Context) {
        let src = self.surface.src.clone();
        let settings = self.controls.clone();
        let name = self.surface.name.clone();
        let rect = self.crop_rect;
        let ratio = [None, Some(1.), Some(4. / 3.), Some(16. / 9.), Some(2. / 3.)][self.crop_ratio];
        self.run(
            ctx,
            "Cropping image and rebuilding material…",
            move || {
                let (width, height) = src.dimensions();
                let mut left = (rect.left().clamp(0., 1.) * width as f32) as u32;
                let mut top = (rect.top().clamp(0., 1.) * height as f32) as u32;
                let mut right = (rect.right().clamp(0., 1.) * width as f32) as u32;
                let mut bottom = (rect.bottom().clamp(0., 1.) * height as f32) as u32;
                if let Some(ratio) = ratio {
                    let mut w = right.saturating_sub(left).max(16);
                    let mut h = bottom.saturating_sub(top).max(16);
                    if w as f32 / h as f32 > ratio {
                        w = (h as f32 * ratio) as u32;
                    } else {
                        h = (w as f32 / ratio) as u32;
                    }
                    left = left.saturating_add(right.saturating_sub(left).saturating_sub(w) / 2);
                    top = top.saturating_add(bottom.saturating_sub(top).saturating_sub(h) / 2);
                    right = left.saturating_add(w).min(width);
                    bottom = top.saturating_add(h).min(height);
                }
                if right.saturating_sub(left) < 16 || bottom.saturating_sub(top) < 16 {
                    return Err("Crop must be at least 16 × 16 pixels.".into());
                }
                let cropped = Arc::new(
                    image::imageops::crop_imm(&*src, left, top, right - left, bottom - top)
                        .to_image(),
                );
                let maps = Arc::new(material::build(&cropped, &settings));
                Ok(Job::Surface(Surface {
                    src: cropped,
                    maps,
                    name,
                    settings,
                }))
            },
        );
    }
    fn refresh(&mut self, ctx: &egui::Context) {
        if self.texture.is_some() {
            return;
        }
        let img = if self.view == 2 {
            preview::sphere(
                &self.surface.maps,
                self.angle,
                self.surface.settings.dx,
                [self.orbit.x, self.orbit.y],
            )
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
        let mut reset_library_scroll = false;
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
                        self.crop_texture = None;
                        self.crop_rect = egui::Rect::from_min_max(
                            egui::pos2(0.08, 0.08),
                            egui::pos2(0.92, 0.92),
                        );
                        self.chips.clear();
                        self.status = lang(self.french, "Material ready", "Matériau prêt").into()
                    }
                    Ok(Job::Saved(p)) => self.status = format!("Exported to {}", p.display()),
                    Ok(Job::Assets(assets)) => {
                        self.assets = assets;
                        self.library_thumbs.clear();
                        let (tx, rx) = mpsc::channel();
                        self.library_previews = Some(rx);
                        let assets = self.assets.clone();
                        let ctx = ctx.clone();
                        thread::spawn(move || {
                            library::load_previews(&assets, |id, image| {
                                let received = tx.send((id, image)).is_ok();
                                ctx.request_repaint();
                                received
                            });
                        });
                        reset_library_scroll = true;
                        self.selected_asset = (!self.assets.is_empty()).then_some(0);
                        self.status = format!("{} textures found", self.assets.len());
                    }
                    Ok(Job::Downloaded(path)) => {
                        self.library_tab = false;
                        self.crop_tab = false;
                        self.view = 1;
                        self.generate(ctx, Some(path));
                    }
                    Err(e) => self.status = e,
                }
            }
        }
        if let Some(previews) = &self.library_previews {
            loop {
                match previews.try_recv() {
                    Ok((id, Some(image))) => {
                        let color = egui::ColorImage::from_rgba_unmultiplied(
                            [image.width() as usize, image.height() as usize],
                            image.as_raw(),
                        );
                        let handle = ctx.load_texture(
                            format!("asset-preview-{id}"),
                            color,
                            egui::TextureOptions::LINEAR,
                        );
                        self.library_thumbs.insert(id, handle);
                    }
                    Ok((_, None)) => {}
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.library_previews = None;
                        break;
                    }
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
                ui.label(
                    RichText::new(lang(self.french, "Material studio", "Atelier de matériaux"))
                        .color(MUTED),
                );
                if ui
                    .add_sized(
                        Vec2::new(34., 22.),
                        egui::Button::new(if self.french { "FR" } else { "EN" }),
                    )
                    .clicked()
                {
                    self.french = !self.french;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !busy && !dirty,
                            egui::Button::new(
                                RichText::new(lang(
                                    self.french,
                                    "Export PBR set",
                                    "Exporter le lot PBR",
                                ))
                                .strong()
                                .color(Color32::BLACK),
                            )
                            .fill(ACCENT),
                        )
                        .on_hover_text(lang(
                            self.french,
                            "Write seven PNG maps into a new folder",
                            "Enregistrer les sept cartes PNG dans un nouveau dossier",
                        ))
                        .clicked()
                    {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title(lang(
                                self.french,
                                "Choose parent folder for material",
                                "Choisir le dossier de destination",
                            ))
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
                    if ui
                        .add_enabled(
                            !busy && !dirty,
                            egui::Button::new(lang(
                                self.french,
                                "Export as Blender asset",
                                "Exporter comme asset Blender",
                            )),
                        )
                        .clicked()
                    {
                        self.show_blender_export = true;
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
        if self.show_blender_export {
            egui::Window::new(lang(
                self.french,
                "Blender material asset",
                "Asset matériau Blender",
            ))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(lang(
                    self.french,
                    "Choose texture mapping",
                    "Choisis le type de mapping",
                ));
                egui::ComboBox::from_id_salt("blender-mapping")
                    .selected_text(["UV", "Cube projection", "Triplanar projection"][self.mapping])
                    .show_ui(ui, |ui| {
                        for (i, label) in ["UV", "Cube projection", "Triplanar projection"]
                            .iter()
                            .enumerate()
                        {
                            ui.selectable_value(&mut self.mapping, i, *label);
                        }
                    });
                ui.horizontal(|ui| {
                    if ui.button(lang(self.french, "Cancel", "Annuler")).clicked() {
                        self.show_blender_export = false;
                    }
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(lang(
                                self.french,
                                "Add to Quby Materials",
                                "Ajouter à Quby Materials",
                            ))
                            .fill(ACCENT),
                        )
                        .clicked()
                    {
                        let maps = self.surface.maps.clone();
                        let name = self.surface.name.clone();
                        let mapping = self.mapping;
                        self.show_blender_export = false;
                        self.run(ctx, "Adding Blender material asset…", move || {
                            export_blender(&name, &maps, mapping).map(Job::Saved)
                        });
                    }
                });
            });
        }
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if busy {
                    ui.spinner();
                }
                ui.label(
                    RichText::new(if dirty && !busy {
                        lang(
                            self.french,
                            "Updating material...",
                            "Mise à jour du matériau…",
                        )
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
                    caption(ui, lang(self.french, "SOURCE IMAGE", "IMAGE SOURCE"));
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
                        .add_enabled(
                            !busy,
                            egui::Button::new(lang(
                                self.french,
                                "Import image…",
                                "Importer une image…",
                            )),
                        )
                        .on_hover_text("Or drop an image onto this window")
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter(
                                lang(self.french, "Texture image", "Image de texture"),
                                &["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff"],
                            )
                            .pick_file()
                        {
                            self.generate(ctx, Some(path));
                        }
                    }
                    ui.add_space(6.);
                    ui.separator();
                    caption(
                        ui,
                        lang(self.french, "SEAM CONTROLS", "RÉGLAGES DES JONCTIONS"),
                    );
                    ui.add_enabled_ui(true, |ui| {
                        use material::Method;
                        egui::ComboBox::from_id_salt("seam_method")
                            .selected_text(lang(
                                self.french,
                                self.controls.method.label(),
                                match self.controls.method {
                                    material::Method::Adaptive => "Coupe adaptative",
                                    material::Method::Straight => "Coupe droite",
                                    material::Method::Blend => "Fondu doux",
                                },
                            ))
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
                        ui.small(lang(
                            self.french,
                            self.controls.method.hint(),
                            match self.controls.method {
                                material::Method::Adaptive => {
                                    "Suit les détails similaires dans la zone de chevauchement."
                                }
                                material::Method::Straight => {
                                    "Place une coupe droite dans le chevauchement."
                                }
                                material::Method::Blend => {
                                    "Mélange progressivement toute la zone de chevauchement."
                                }
                            },
                        ));
                        for (label, value) in [
                            (
                                lang(
                                    self.french,
                                    "Horizontal overlap",
                                    "Chevauchement horizontal",
                                ),
                                &mut self.controls.overlap,
                            ),
                            (
                                lang(self.french, "Vertical overlap", "Chevauchement vertical"),
                                &mut self.controls.overlap_y,
                            ),
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
                            ui.label(lang(self.french, "Feather radius", "Rayon de fondu"));
                            ui.add(
                                egui::Slider::new(&mut self.controls.feather, 0.0..=24.)
                                    .suffix(" px"),
                            );
                        }
                        if self.controls.method == Method::Straight {
                            ui.label(lang(
                                self.french,
                                "Cut position in overlap",
                                "Position de la coupe",
                            ));
                            ui.add(
                                egui::Slider::new(&mut self.controls.position, 0.1..=0.9)
                                    .fixed_decimals(2),
                            );
                        }
                        ui.separator();
                        caption(ui, lang(self.french, "PBR CONTROLS", "RÉGLAGES PBR"));
                        for (label, val, range) in [
                            (
                                lang(self.french, "Relief strength", "Intensité du relief"),
                                &mut self.controls.relief,
                                0.0..=10.0,
                            ),
                            (
                                lang(self.french, "Roughness", "Rugosité"),
                                &mut self.controls.rough,
                                0.0..=1.0,
                            ),
                            (
                                lang(self.french, "Metallic", "Métallique"),
                                &mut self.controls.metal,
                                0.0..=1.0,
                            ),
                        ] {
                            ui.label(label);
                            ui.add(egui::Slider::new(val, range).fixed_decimals(2));
                        }
                        ui.checkbox(
                            &mut self.controls.invert,
                            lang(self.french, "Invert height", "Inverser la hauteur"),
                        );
                        ui.checkbox(
                            &mut self.controls.dx,
                            lang(self.french, "DirectX normals (−Y)", "Normales DirectX (−Y)"),
                        );
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
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(!self.crop_tab, lang(self.french, "Material", "Matériau"))
                        .clicked()
                    {
                        self.crop_tab = false;
                        self.library_tab = false;
                    }
                    if ui
                        .selectable_label(self.crop_tab, lang(self.french, "Crop", "Recadrage"))
                        .clicked()
                    {
                        self.crop_tab = true;
                        self.library_tab = false;
                    }
                    if ui.selectable_label(self.library_tab,lang(self.french,"Library","Bibliothèque")).clicked(){self.library_tab=true;self.crop_tab=false;}
                });
                ui.separator();
                if self.library_tab {
                    ui.horizontal(|ui| {
                        ui.heading(lang(self.french, "Texture library", "Bibliothèque de textures"));
                        egui::ComboBox::from_id_salt("library-provider")
                            .selected_text(match self.provider { library::Provider::PolyHaven => "Poly Haven", library::Provider::AmbientCg => "ambientCG" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.provider, library::Provider::PolyHaven, "Poly Haven");
                                ui.selectable_value(&mut self.provider, library::Provider::AmbientCg, "ambientCG");
                            });
                        let search = ui.add(egui::TextEdit::singleline(&mut self.search_text)
                            .hint_text(lang(self.french, "Search materials", "Rechercher des textures"))
                            .desired_width(190.));
                        let enter = search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let button = ui.add_enabled(!busy, egui::Button::new(lang(self.french, "Search", "Rechercher"))).clicked();
                        if !busy && (enter || button) {
                            let provider=self.provider; let query=self.search_text.clone();
                            self.run(ctx, "Searching texture libraries…", move || library::search(provider,&query).map(Job::Assets));
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(lang(self.french,"Resolution","Résolution"));
                        egui::ComboBox::from_id_salt("texture-resolution").selected_text(self.resolution.to_uppercase()).show_ui(ui,|ui|{
                            for resolution in ["1k","2k","4k","8k"] { ui.selectable_value(&mut self.resolution,resolution.to_string(),resolution.to_uppercase()); }
                        });
                        ui.label(RichText::new(lang(self.french,"Powered by Poly Haven · ambientCG materials are CC0","Données Poly Haven · matériaux ambientCG sous licence CC0")).small().color(MUTED));
                    });
                    let mut download_choice = None;
                    let mut results = egui::ScrollArea::vertical()
                        .id_salt("library-results")
                        .auto_shrink([false, false]);
                    if reset_library_scroll {
                        results = results.vertical_scroll_offset(0.0);
                    }
                    results.show(ui, |ui| {
                        let columns = if ui.available_width() > 760. { 3 }
                            else if ui.available_width() > 490. { 2 }
                            else { 1 };
                        for (row, assets) in self.assets.chunks(columns).enumerate() {
                            ui.columns(columns, |panels| {
                                for (column, asset) in assets.iter().enumerate() {
                                    let index = row * columns + column;
                                    panels[column].group(|ui| {
                                        let (rect, response) = ui.allocate_exact_size(
                                            Vec2::new(ui.available_width(), 144.),
                                            egui::Sense::click(),
                                        );
                                        if let Some(image) = self.library_thumbs.get(&asset.id) {
                                            let size = image.size_vec2();
                                            let scale = (rect.width() / size.x).min(rect.height() / size.y);
                                            ui.painter().image(
                                                image.id(),
                                                egui::Rect::from_center_size(rect.center(), size * scale),
                                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                                                Color32::WHITE,
                                            );
                                            if response.clicked() {
                                                self.selected_asset = Some(index);
                                            }
                                        }
                                        let selected = self.selected_asset == Some(index);
                                        let title = RichText::new(&asset.name).strong();
                                        ui.add(egui::Label::new(if selected { title.color(ACCENT) } else { title }).truncate())
                                            .on_hover_text(&asset.name);
                                        ui.add(egui::Label::new(RichText::new(&asset.id).small()).truncate());
                                        if ui.add_enabled(!busy, egui::Button::new(lang(self.french, "Download & tile", "Télécharger et carreler"))).clicked() {
                                            download_choice = Some(asset.clone());
                                        }
                                    });
                                }
                            });
                        }
                    });
                    if let Some(asset)=download_choice {
                        let resolution=self.resolution.clone();
                        let folder=std::env::var("USERPROFILE").map(PathBuf::from).unwrap_or_else(|_|std::env::temp_dir()).join("Downloads").join("quby-seemdissolver").join("Textures");
                        self.run(ctx,lang(self.french,"Downloading texture maps…","Téléchargement des textures…"),move||library::download(&asset,&folder,&resolution).map(Job::Downloaded));
                    }
                } else if self.crop_tab {
                    caption(
                        ui,
                        lang(self.french, "CROP SOURCE IMAGE", "RECADRER L’IMAGE SOURCE"),
                    );
                    ui.horizontal(|ui| {
                        ui.label(lang(self.french, "Aspect ratio", "Proportions"));
                        let ratio_changed = egui::ComboBox::from_id_salt("crop_ratio")
                            .selected_text(["Libre", "1:1", "4:3", "16:9", "2:3"][self.crop_ratio])
                            .show_ui(ui, |ui| {
                                for (i, label) in
                                    ["Libre", "1:1", "4:3", "16:9", "2:3"].iter().enumerate()
                                {
                                    ui.selectable_value(&mut self.crop_ratio, i, *label);
                                }
                            })
                            .response
                            .changed();
                        if ratio_changed && self.crop_ratio != 0 {
                            let ratios = [1., 1., 4. / 3., 16. / 9., 2. / 3.];
                            let image_ratio =
                                self.surface.src.width() as f32 / self.surface.src.height() as f32;
                            let crop_ratio = ratios[self.crop_ratio] / image_ratio;
                            let mut width = self.crop_rect.width().min(0.9);
                            let mut height = width / crop_ratio;
                            if height > 0.9 {
                                height = 0.9;
                                width = height * crop_ratio;
                            }
                            let center = self.crop_rect.center();
                            let left = (center.x - width / 2.).clamp(0., 1. - width);
                            let top = (center.y - height / 2.).clamp(0., 1. - height);
                            self.crop_rect = egui::Rect::from_min_size(
                                egui::pos2(left, top),
                                egui::vec2(width, height),
                            );
                        }
                        if ui
                            .button(lang(self.french, "Reset", "Réinitialiser"))
                            .clicked()
                        {
                            self.crop_rect = egui::Rect::from_min_max(
                                egui::pos2(0.08, 0.08),
                                egui::pos2(0.92, 0.92),
                            );
                        }
                    });
                    if self.crop_texture.is_none() {
                        let img = image::DynamicImage::ImageRgb8((*self.surface.src).clone())
                            .thumbnail(1600, 1000)
                            .to_rgb8();
                        self.crop_texture = Some(texture(ctx, "crop-source", &img));
                    }
                    let tex = self.crop_texture.as_ref().unwrap();
                    let (viewport, _) = ui.allocate_exact_size(
                        Vec2::new(
                            ui.available_width(),
                            (ui.available_height() - 100.).max(100.),
                        ),
                        egui::Sense::click_and_drag(),
                    );
                    let size = tex.size_vec2()
                        * ((viewport.width() / tex.size_vec2().x)
                            .min(viewport.height() / tex.size_vec2().y));
                    let image_rect = egui::Rect::from_center_size(viewport.center(), size);
                    ui.painter().image(
                        tex.id(),
                        image_rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                        Color32::WHITE,
                    );
                    let selected = egui::Rect::from_min_max(
                        image_rect.min + size * self.crop_rect.min.to_vec2(),
                        image_rect.min + size * self.crop_rect.max.to_vec2(),
                    );
                    let painter = ui.painter_at(image_rect);
                    for outside in [
                        egui::Rect::from_min_max(
                            image_rect.min,
                            egui::pos2(image_rect.right(), selected.top()),
                        ),
                        egui::Rect::from_min_max(
                            egui::pos2(image_rect.left(), selected.bottom()),
                            image_rect.max,
                        ),
                        egui::Rect::from_min_max(
                            egui::pos2(image_rect.left(), selected.top()),
                            egui::pos2(selected.left(), selected.bottom()),
                        ),
                        egui::Rect::from_min_max(
                            egui::pos2(selected.right(), selected.top()),
                            egui::pos2(image_rect.right(), selected.bottom()),
                        ),
                    ] {
                        painter.rect_filled(outside, 0., Color32::from_black_alpha(165));
                    }
                    painter.rect_stroke(
                        selected,
                        0.,
                        egui::Stroke::new(2., ACCENT),
                        egui::StrokeKind::Inside,
                    );
                    let handle =
                        egui::Rect::from_center_size(selected.right_bottom(), Vec2::splat(20.));
                    painter.rect_filled(handle, 3., ACCENT);
                    let move_response =
                        ui.interact(selected, ui.id().with("crop-move"), egui::Sense::drag());
                    if move_response.dragged()
                        && !handle
                            .contains(move_response.interact_pointer_pos().unwrap_or_default())
                    {
                        let delta = move_response.drag_delta() / size;
                        let width = self.crop_rect.width();
                        let height = self.crop_rect.height();
                        let left = (self.crop_rect.left() + delta.x).clamp(0., 1. - width);
                        let top = (self.crop_rect.top() + delta.y).clamp(0., 1. - height);
                        self.crop_rect = egui::Rect::from_min_size(
                            egui::pos2(left, top),
                            egui::vec2(width, height),
                        );
                    }
                    let resize_response =
                        ui.interact(handle, ui.id().with("crop-resize"), egui::Sense::drag());
                    if resize_response.dragged() {
                        let delta = resize_response.drag_delta() / size;
                        let mut right = (self.crop_rect.right() + delta.x)
                            .clamp(self.crop_rect.left() + 0.05, 1.);
                        let mut bottom = (self.crop_rect.bottom() + delta.y)
                            .clamp(self.crop_rect.top() + 0.05, 1.);
                        if self.crop_ratio != 0 {
                            let ratios = [1., 1., 4. / 3., 16. / 9., 2. / 3.];
                            let crop_ratio = ratios[self.crop_ratio]
                                / (self.surface.src.width() as f32
                                    / self.surface.src.height() as f32);
                            if (right - self.crop_rect.left()) / (bottom - self.crop_rect.top())
                                > crop_ratio
                            {
                                right = self.crop_rect.left()
                                    + (bottom - self.crop_rect.top()) * crop_ratio;
                            } else {
                                bottom = self.crop_rect.top()
                                    + (right - self.crop_rect.left()) / crop_ratio;
                            }
                        }
                        self.crop_rect.max = egui::pos2(right, bottom);
                    }
                    let selection = if self.french {
                        format!(
                            "Sélection : {:.0}% × {:.0}% de l’image",
                            self.crop_rect.width() * 100.,
                            self.crop_rect.height() * 100.
                        )
                    } else {
                        format!(
                            "Selection: {:.0}% × {:.0}% of image",
                            self.crop_rect.width() * 100.,
                            self.crop_rect.height() * 100.
                        )
                    };
                    ui.label(selection);
                    ui.horizontal(|ui| {
                        ui.label(lang(
                            self.french,
                            "Drag the selection to move it. Drag the cyan corner to resize.",
                            "Déplace la zone. Fais glisser le coin cyan pour la redimensionner.",
                        ));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    !busy,
                                    egui::Button::new(lang(
                                        self.french,
                                        "Crop and apply",
                                        "Recadrer et appliquer",
                                    ))
                                    .fill(ACCENT)
                                    .stroke(egui::Stroke::NONE),
                                )
                                .clicked()
                            {
                                self.generate_crop(ctx);
                            }
                        });
                    });
                } else {
                    ui.horizontal_wrapped(|ui| {
                        for (i, name) in [
                            lang(self.french, "Single tile", "Tuile seule"),
                            "3 × 3 repeat",
                            lang(self.french, "Lit sphere", "Sphère éclairée"),
                        ]
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
                            .small_button(lang(self.french, "Fit", "Ajuster"))
                            .on_hover_text(lang(
                                self.french,
                                "Reset zoom and pan",
                                "Réinitialiser zoom et cadrage",
                            ))
                            .clicked()
                        {
                            self.cameras[self.view] = Camera::default();
                        }
                    });
                    if self.view == 1 {
                        ui.checkbox(
                            &mut self.grid,
                            lang(
                                self.french,
                                "Show tile boundaries",
                                "Afficher les limites des tuiles",
                            ),
                        );
                    }
                    if self.view == 2 {
                        ui.horizontal(|ui| {
                            ui.small(lang(self.french, "Light angle", "Angle de lumière"));
                            if ui
                                .add(
                                    egui::Slider::new(&mut self.angle, 0.05..=3.09)
                                        .show_value(false),
                                )
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
                    let tex = self.texture.as_ref().unwrap().clone();
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
                    if self.view == 2 && ui.input(|i|i.pointer.button_down(egui::PointerButton::Middle)) {
                        let delta=ui.input(|i|i.pointer.delta());
                        self.orbit+=Vec2::new(delta.x*0.012,delta.y*0.012);
                        self.texture=None;
                    }
                    if response.dragged() {
                        camera.offset += ui.input(|i| i.pointer.delta());
                    }
                    if response.double_clicked() {
                        *camera = Camera::default();
                    }
                    camera.constrain(size, rect.size());
                response.on_hover_text(if self.view==2 {lang(self.french,"Middle-drag to orbit · wheel to zoom · double-click to fit","Clic-molette pour orbiter · molette pour zoomer · double-clic pour ajuster")} else {lang(self.french,"Wheel to zoom · drag to pan · double-click to fit","Molette pour zoomer · glisser pour déplacer · double-clic pour ajuster")});
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
                    caption(ui, lang(self.french, "TEXTURE MAPS", "CARTES DE TEXTURE"));
                    if self.chips.is_empty() {
                        self.chips = self
                            .surface
                            .maps
                            .iter()
                            .enumerate()
                            .map(|(i, m)| {
                                let (w, h) = m.dimensions();
                                let d = w.min(h);
                                let crop =
                                    image::imageops::crop_imm(m, (w - d) / 2, (h - d) / 2, d, d);
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
                }
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
