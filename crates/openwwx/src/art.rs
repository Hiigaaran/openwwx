//! Card art found in the card folder. Any image whose file name matches a
//! card's source id (an OCTGN card GUID), its id or its title is used as that
//! card's picture; the image is the whole scanned card.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui::{ColorImage, Context, Id, TextureHandle, TextureOptions};
use rawdeal_core::{CardDef, SuperstarDef};

const EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

#[derive(Default)]
pub struct Art {
    /// Lowercase file stem to path.
    files: HashMap<String, PathBuf>,
    /// Loaded textures by file stem; `None` if the file could not be read.
    cache: HashMap<String, Option<TextureHandle>>,
}

type Shared = Arc<Mutex<Art>>;

fn id() -> Id {
    Id::new("openwwx-art")
}

/// Indexes the images under `dir` (and its subfolders) for this session.
pub fn install(ctx: &Context, dir: Option<&Path>) -> usize {
    let mut art = Art::default();
    if let Some(dir) = dir {
        index(dir, 0, &mut art.files);
    }
    let n = art.files.len();
    ctx.data_mut(|d| d.insert_temp::<Shared>(id(), Arc::new(Mutex::new(art))));
    n
}

fn index(dir: &Path, depth: usize, files: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < 6 {
                index(&path, depth + 1, files);
            }
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_lowercase);
        if ext.is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            files.entry(stem.to_lowercase()).or_insert(path);
        }
    }
}

/// Names an image may have for a card with this id, title and source id.
fn keys(id: &str, title: &str, source: Option<&str>) -> Vec<String> {
    let mut keys: Vec<String> = source.into_iter().map(str::to_lowercase).collect();
    let id = id.to_lowercase();
    keys.push(id.replace('/', "-"));
    keys.push(id.replace('/', "_"));
    let title = title.to_lowercase();
    keys.push(title.replace(' ', "_"));
    keys.push(title.replace(' ', "-"));
    keys.push(title);
    keys
}

fn lookup(ctx: &Context, keys: &[String]) -> Option<TextureHandle> {
    let shared = ctx.data(|d| d.get_temp::<Shared>(id()))?;
    let mut art = shared.lock().ok()?;
    let key = keys.iter().find(|k| art.files.contains_key(*k))?.clone();
    if let Some(cached) = art.cache.get(&key) {
        return cached.clone();
    }
    let texture = load(ctx, &art.files[&key], &key);
    art.cache.insert(key, texture.clone());
    texture
}

fn load(ctx: &Context, path: &Path, name: &str) -> Option<TextureHandle> {
    let bytes = std::fs::read(path).ok()?;
    let image = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let image = ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture(format!("art-{name}"), image, TextureOptions::LINEAR))
}

pub fn card(ctx: &Context, def: &CardDef) -> Option<TextureHandle> {
    lookup(ctx, &keys(&def.id, &def.title, def.source_id.as_deref()))
}

pub fn superstar(ctx: &Context, star: &SuperstarDef) -> Option<TextureHandle> {
    lookup(ctx, &keys(&star.id, &star.name, star.source_id.as_deref()))
}
