use reqwest::blocking::Client;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Provider {
    PolyHaven,
    AmbientCg,
}
#[derive(Clone)]
pub struct Asset {
    pub provider: Provider,
    pub id: String,
    pub name: String,
    pub image: String,
    pub download: String,
}
fn trusted_url(url: &str) -> bool {
    [
        "https://api.polyhaven.com/",
        "https://cdn.polyhaven.com/",
        "https://dl.polyhaven.org/",
        "https://ambientcg.com/",
        "https://www.ambientcg.com/",
        "https://acg-media.struffelproductions.com/",
    ]
    .iter()
    .any(|prefix| url.starts_with(prefix))
}
pub fn search(provider: Provider, query: &str) -> Result<Vec<Asset>, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(35))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    match provider {
        Provider::PolyHaven => {
            let assets: Value = client
                .get("https://api.polyhaven.com/assets")
                .query(&[("t", "textures")])
                .send()
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json()
                .map_err(|e| e.to_string())?;
            let q = query.to_lowercase();
            for (id, v) in assets
                .as_object()
                .ok_or("Poly Haven returned unexpected data")?
            {
                let name = v.get("name").and_then(Value::as_str).unwrap_or(id);
                let tags = v
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                if !q.is_empty() && !format!("{name} {id} {tags}").to_lowercase().contains(&q) {
                    continue;
                }
                let thumb = v
                    .get("thumbnail_url")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                out.push(Asset {
                    provider,
                    id: id.clone(),
                    name: name.into(),
                    image: thumb.into(),
                    download: format!("https://api.polyhaven.com/files/{id}"),
                });
                if out.len() >= 48 {
                    break;
                }
            }
        }
        Provider::AmbientCg => {
            let v: Value = client
                .get("https://ambientCG.com/api/v3/assets")
                .query(&[
                    ("type", "material"),
                    ("q", query),
                    ("limit", "48"),
                    ("include", "title,thumbnails,downloads"),
                ])
                .send()
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json()
                .map_err(|e| e.to_string())?;
            for item in v
                .get("assets")
                .and_then(Value::as_array)
                .ok_or("ambientCG returned unexpected data")?
            {
                let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
                let name = item.get("title").and_then(Value::as_str).unwrap_or(id);
                let image = item
                    .get("thumbnails")
                    .and_then(|x| x.get("256-PNG"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let download = item
                    .get("downloads")
                    .and_then(Value::as_array)
                    .and_then(|a| {
                        a.iter()
                            .find(|d| d.get("attributes").and_then(Value::as_str) == Some("2K-PNG"))
                            .or_else(|| a.first())
                    })
                    .and_then(|d| d.get("url"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                out.push(Asset {
                    provider,
                    id: id.into(),
                    name: name.into(),
                    image: image.into(),
                    download: download.into(),
                });
            }
        }
    }
    Ok(out)
}

const USER_AGENT: &str = concat!(
    "quby-seemdissolver/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/wasnas/quby-seemdissolver)"
);

pub fn load_previews(
    assets: &[Asset],
    publish: impl Fn(String, Option<image::RgbaImage>) -> bool + Sync,
) {
    let Ok(client) = Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(USER_AGENT)
        .build()
    else {
        return;
    };
    std::thread::scope(|scope| {
        for lane in 0..assets.len().min(6) {
            let client = &client;
            let publish = &publish;
            scope.spawn(move || {
                for asset in assets.iter().skip(lane).step_by(6) {
                    let image = if trusted_url(&asset.image) {
                        client
                            .get(&asset.image)
                            .send()
                            .and_then(|response| response.error_for_status())
                            .and_then(|response| response.bytes())
                            .ok()
                            .and_then(|bytes| image::load_from_memory(&bytes).ok())
                            .map(|image| image.into_rgba8())
                    } else {
                        None
                    };
                    if !publish(asset.id.clone(), image) {
                        break;
                    }
                }
            });
        }
    });
}
pub fn download(
    asset: &Asset,
    folder: &std::path::Path,
    resolution: &str,
) -> Result<std::path::PathBuf, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(240))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let safe_id: String = asset
        .id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    let download_url = match asset.provider {
        Provider::AmbientCg => format!(
            "https://ambientcg.com/get?file={}_{}-PNG.zip",
            safe_id,
            resolution.to_ascii_uppercase()
        ),
        Provider::PolyHaven => asset.download.clone(),
    };
    if !trusted_url(&download_url) {
        return Err("Catalogue returned an untrusted download URL".into());
    }
    let path = folder.join(safe_id);
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    match asset.provider {
        Provider::AmbientCg => {
            let bytes = client
                .get(&download_url)
                .send()
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .bytes()
                .map_err(|e| e.to_string())?;
            let mut archive =
                zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
            let mut color = None;
            for i in 0..archive.len() {
                let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
                let Some(name) = file
                    .enclosed_name()
                    .and_then(|p| p.file_name().map(|n| n.to_owned()))
                else {
                    continue;
                };
                let ext = name.to_string_lossy().to_ascii_lowercase();
                if !["png", "jpg", "jpeg", "webp", "tif", "tiff"]
                    .iter()
                    .any(|e| ext.ends_with(e))
                {
                    continue;
                }
                if file.size() > 128 * 1024 * 1024 {
                    continue;
                }
                let out = path.join(&name);
                let mut dest = std::fs::File::create(&out).map_err(|e| e.to_string())?;
                std::io::copy(&mut file, &mut dest).map_err(|e| e.to_string())?;
                let stem = out.file_stem().unwrap_or_default().to_string_lossy();
                let map = stem.rsplit(['_', '-', ' ']).next().unwrap_or_default();
                if color.is_none()
                    && ["color", "colour", "basecolor", "albedo", "diffuse", "diff"]
                        .iter()
                        .any(|role| map.eq_ignore_ascii_case(role))
                {
                    color = Some(out);
                }
            }
            color.ok_or_else(|| format!("ambientCG asset has no color image at {resolution}"))
        }
        Provider::PolyHaven => {
            let data: Value = client
                .get(&download_url)
                .send()
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json()
                .map_err(|e| e.to_string())?;
            let mut color = None;
            for map in ["Diffuse", "nor_gl", "Rough", "AO", "Displacement"] {
                let Some(file) = data
                    .get(map)
                    .and_then(|v| v.get(resolution))
                    .and_then(|v| v.get("png").or_else(|| v.get("jpg")))
                else {
                    continue;
                };
                let Some(url) = file.get("url").and_then(Value::as_str) else {
                    continue;
                };
                if !trusted_url(url) {
                    continue;
                }
                let name = url
                    .rsplit('/')
                    .next()
                    .unwrap_or(map)
                    .split('?')
                    .next()
                    .unwrap_or(map);
                let bytes = client
                    .get(url)
                    .send()
                    .map_err(|e| e.to_string())?
                    .error_for_status()
                    .map_err(|e| e.to_string())?
                    .bytes()
                    .map_err(|e| e.to_string())?;
                std::fs::write(path.join(name), &bytes).map_err(|e| e.to_string())?;
                if map == "Diffuse" {
                    color = Some(path.join(name));
                }
            }
            color.ok_or_else(|| format!("Poly Haven asset has no color image at {resolution}"))
        }
    }
}
