//! Android devices' art (`crates/plugin-mobile/command/src/live.rs`), from
//! the Android Open Source Project, where Android Studio's comes from.
//!
//! For each device, such as `pixel_10`, a folder holds the frame's image, the
//! screen's mask and a `layout` in the emulator's skin format that places the
//! screen in the frame. A foldable's folder, such as `pixel_fold`, holds one
//! such folder per posture: `default` open and `closed` folded.
//!
//! A Host downloads a device's folder the first time it opens the device, at
//! the commit the pin names, and keeps it once each file matches the pin's
//! SHA-256 (`art_pin.rs`).
//!
//! The art is drawn as Android Studio's emulator window draws it
//! (`SkinDefinition` in its `adt-ui`):
//!
//! - the frame's image around the screen and never over it;
//! - the mask over the screen's corners;
//! - the device's buttons;
//! - the whole turned with the picture.

use std::path::{Path, PathBuf};

use image::{RgbaImage, imageops};
use tokio_util::sync::CancellationToken;

use super::art_pin::{COMMIT, DEVICES};
use crate::driver::{FrameArea, PixelRect};

/// Where AOSP serves the repository that holds the art.
const REPOSITORY: &str = "https://android.googlesource.com/platform/tools/adt/idea";
/// The art's folder in the repository.
const ROOT: &str = "artwork/resources/device-art-resources";
/// The most bytes a device's archive may have: a foldable's two postures
/// are under a megabyte, and the archive is read into memory.
const ARCHIVE_BYTES: u64 = 16 * 1024 * 1024;

/// The devices' art a Host keeps, in a folder per pinned commit.
pub struct Store {
    /// The folder of the pinned commit's devices.
    skins: PathBuf,
    /// One download of a device's art at a time.
    downloading: tokio::sync::Mutex<()>,
}

impl Store {
    /// The art kept in `directory`.
    pub fn new(directory: &Path) -> Self {
        Self {
            skins: directory.join(COMMIT),
            downloading: tokio::sync::Mutex::new(()),
        }
    }

    /// The art of `device`, such as `pixel_10`, downloaded when this Host
    /// has none; none for a device AOSP has no art for. A download that
    /// fails is said in the log and tried again the next time.
    pub async fn device(&self, device: &str) -> Option<Folders> {
        let (_, files) = DEVICES.iter().find(|(name, _)| *name == device)?;
        let folder = self.skins.join(device);
        if let Some(folders) = folders(&folder) {
            return Some(folders);
        }
        let _downloading = self.downloading.lock().await;
        if let Some(folders) = folders(&folder) {
            return Some(folders);
        }
        match download(&self.skins, device, files).await {
            Ok(()) => {
                tracing::info!(event = "android_art.downloaded", device, "an Android device's art was downloaded");
                folders(&folder)
            }
            Err(error) => {
                tracing::warn!(event = "android_art.unavailable", device, %error, fallback = "drawn", "an Android device's art did not download");
                None
            }
        }
    }
}

/// The folders of `device`'s art in `folder`, once it is kept.
fn folders(folder: &Path) -> Option<Folders> {
    let art = |folder: PathBuf| folder.join("layout").is_file().then_some(folder);
    // A foldable's art is a folder per posture.
    match art(folder.join("default")) {
        Some(open) => Some(Folders { open, closed: art(folder.join("closed")) }),
        None => art(folder.to_owned()).map(|open| Folders { open, closed: None }),
    }
}

/// Downloads `device`'s archive into `skins`, keeping its folder only once
/// each of `files` matches its SHA-256.
async fn download(skins: &Path, device: &str, files: &[(&str, &str)]) -> Result<(), String> {
    let url = format!("{REPOSITORY}/+archive/{COMMIT}/{ROOT}/{device}.tar.gz");
    let client = demi_shared_artifacts::download_client().map_err(|error| error.to_string())?;
    let request = demi_shared_artifacts::Download {
        limit: ARCHIVE_BYTES,
        ..demi_shared_artifacts::Download::new(&url)
    };
    let (received, archive) = demi_shared_artifacts::download_bytes(&client, request, &CancellationToken::new())
        .await
        .map_err(|error| format!("{url}: {error}"))?;
    received.plain().map_err(|error| format!("{url}: {error}"))?;
    let skins = skins.to_owned();
    let device = device.to_owned();
    let files: Vec<(String, String)> = files.iter().map(|(path, sha256)| ((*path).to_owned(), (*sha256).to_owned())).collect();
    tokio::task::spawn_blocking(move || keep(&skins, &device, &files, &archive))
        .await
        .map_err(|error| error.to_string())?
}

/// Writes the pinned `files` of `archive` into `skins/device`, through a
/// folder beside it, so a program killed midway leaves no device half kept.
fn keep(skins: &Path, device: &str, files: &[(String, String)], archive: &[u8]) -> Result<(), String> {
    let mut found = std::collections::HashMap::new();
    let mut entries = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in entries.entries().map_err(|error| error.to_string())? {
        let mut entry = entry.map_err(|error| error.to_string())?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(|error| error.to_string())?.to_string_lossy().into_owned();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).map_err(|error| error.to_string())?;
        found.insert(path, bytes);
    }
    std::fs::create_dir_all(skins).map_err(|error| format!("{}: {error}", skins.display()))?;
    let staging = tempfile::Builder::new()
        .prefix(&format!(".{device}-"))
        .tempdir_in(skins)
        .map_err(|error| format!("{}: {error}", skins.display()))?;
    for (path, sha256) in files {
        let bytes = found.get(path).ok_or_else(|| format!("the archive has no {path}"))?;
        let digest: String = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes)
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if digest != *sha256 {
            return Err(format!("{path} does not match its pinned hash"));
        }
        let target = staging.path().join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        std::fs::write(&target, bytes).map_err(|error| format!("{}: {error}", target.display()))?;
    }
    let folder = skins.join(device);
    std::fs::rename(staging.keep(), &folder).map_err(|error| format!("{}: {error}", folder.display()))
}

/// The folders of a device's art: open, which is every device's, and
/// closed, a foldable's folded.
#[derive(Debug, Clone)]
pub struct Folders {
    pub open: PathBuf,
    pub closed: Option<PathBuf>,
}

/// A block of a skin layout: its values and its blocks, by name, in order.
#[derive(Debug, Default)]
struct Block {
    values: Vec<(String, String)>,
    blocks: Vec<(String, Block)>,
}

impl Block {
    /// Reads the layout's text: `name value` and `name { … }`, separated
    /// by white space, `#` starting a comment line.
    fn parse(text: &str) -> Self {
        let mut tokens = text
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .flat_map(str::split_whitespace);
        Self::read(&mut tokens)
    }

    fn read<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Self {
        let mut block = Self::default();
        while let Some(key) = tokens.next() {
            if key == "}" {
                break;
            }
            let Some(value) = tokens.next() else { break };
            if value == "{" {
                block.blocks.push((key.to_owned(), Self::read(tokens)));
            } else {
                block.values.push((key.to_owned(), value.to_owned()));
            }
        }
        block
    }

    fn block(&self, name: &str) -> Option<&Block> {
        self.blocks.iter().find(|(each, _)| each == name).map(|(_, block)| block)
    }

    /// The value at a dotted `path`, such as `display.width`.
    fn value(&self, path: &str) -> Option<&str> {
        let (blocks, name) = path.rsplit_once('.').map_or(("", path), |(blocks, name)| (blocks, name));
        let block = blocks.split('.').filter(|part| !part.is_empty()).try_fold(self, |block, part| block.block(part))?;
        block.values.iter().find(|(each, _)| each == name).map(|(_, value)| value.as_str())
    }

    fn number(&self, path: &str) -> Option<i64> {
        self.value(path)?.parse().ok()
    }
}

/// A part of the layout whose images frame the screen.
struct Part<'a> {
    block: &'a Block,
    /// Where the part lies from the screen's top left corner, in the art's
    /// pixels.
    x: i64,
    y: i64,
}

/// What the layout says: the screen's size and the part around it.
struct Layout<'a> {
    width: u32,
    height: u32,
    part: Part<'a>,
}

/// The layout of the device upright, as Android Studio picks it: the
/// screen's size from the `device` part, and the first layout that places
/// that part unturned beside a part of images.
fn layout(skin: &Block) -> Result<Layout<'_>, String> {
    let parts = skin.block("parts").ok_or("the layout has no parts")?;
    let display = ["device", "primary"]
        .iter()
        .find_map(|name| parts.block(name))
        .ok_or("the layout has no device part")?;
    let size = |name| display.number(name).and_then(|size| u32::try_from(size).ok()).filter(|size| *size > 0);
    let (width, height) = size("display.width")
        .zip(size("display.height"))
        .ok_or("the layout gives no screen size")?;
    for (_, placed) in &skin.block("layouts").ok_or("the layout has no layouts")?.blocks {
        let mut device = None;
        let mut part = None;
        for (_, each) in &placed.blocks {
            let Some(name) = each.value("name") else { continue };
            let at = (each.number("x").unwrap_or(0), each.number("y").unwrap_or(0));
            if name == "device" || name == "primary" {
                device = Some((at, each.number("rotation").unwrap_or(0)));
            } else if part.is_none() {
                part = parts.block(name).map(|block| (block, at));
            }
        }
        if let (Some((device, 0)), Some((block, at))) = (device, part) {
            return Ok(Layout {
                width,
                height,
                part: Part { block, x: at.0 - device.0, y: at.1 - device.1 },
            });
        }
    }
    Err("no layout shows the device upright".into())
}

/// The device's art drawn once, upright, in its own pixels: the frame with
/// its mask and buttons, transparent where the picture shows.
#[derive(Debug, Clone)]
pub struct Art {
    pub image: RgbaImage,
    /// Where the screen is in it.
    pub screen: (u32, u32, u32, u32),
}

/// An image of the art's folder that the layout names.
fn image_at(folder: &Path, block: &Block, path: &str) -> Result<Option<RgbaImage>, String> {
    let Some(name) = block.value(path) else { return Ok(None) };
    // The layout names files of its own folder.
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return Err(format!("the layout names {name}, outside its folder"));
    }
    let file = folder.join(name);
    image::open(&file)
        .map(|image| Some(image.into_rgba8()))
        .map_err(|error| format!("{} does not read: {error}", file.display()))
}

impl Art {
    /// The art in `folder`.
    pub fn load(folder: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(folder.join("layout")).map_err(|error| format!("its layout does not read: {error}"))?;
        let skin = Block::parse(&text);
        let layout = layout(&skin)?;
        let part = &layout.part;
        let background = image_at(folder, part.block, "background.image")?;
        let mut pieces: Vec<(RgbaImage, i64, i64)> = Vec::new();
        let (width, height) = (i64::from(layout.width), i64::from(layout.height));
        // The screen's own pixels of the frame's image, which Android
        // Studio uses as the mask when they are clear at the screen's centre,
        // as on a frame with its screen cut out; the mask's file otherwise.
        let clear_centre = background.as_ref().is_some_and(|image| {
            let (x, y) = (width / 2 - part.x, height / 2 - part.y);
            u32::try_from(x).ok().zip(u32::try_from(y).ok()).and_then(|(x, y)| image.get_pixel_checked(x, y)).is_none_or(|pixel| pixel[3] == 0)
        });
        if let Some(mut image) = background {
            // The frame's image never covers the screen, unless its own
            // screen is the mask.
            if !clear_centre {
                for y in 0..layout.height {
                    for x in 0..layout.width {
                        let inside = (i64::from(x) - part.x, i64::from(y) - part.y);
                        let (Ok(x), Ok(y)) = (u32::try_from(inside.0), u32::try_from(inside.1)) else { continue };
                        if let Some(pixel) = image.get_pixel_mut_checked(x, y) {
                            *pixel = image::Rgba([0, 0, 0, 0]);
                        }
                    }
                }
            }
            pieces.push((image, part.x, part.y));
        }
        if !clear_centre && let Some(mask) = image_at(folder, part.block, "foreground.mask")? {
            let mask = if mask.dimensions() == (layout.width, layout.height) {
                mask
            } else {
                imageops::resize(&mask, layout.width, layout.height, imageops::FilterType::Triangle)
            };
            pieces.push((mask, 0, 0));
        }
        if let Some(buttons) = part.block.block("buttons") {
            for (_, button) in &buttons.blocks {
                let (Some(x), Some(y)) = (button.number("x"), button.number("y")) else { continue };
                if let Some(image) = image_at(folder, button, "image")? {
                    pieces.push((image, part.x + x, part.y + y));
                }
            }
        }
        if pieces.is_empty() {
            return Err("the layout names no images".into());
        }
        // The whole art, cropped to what shows, as Android Studio crops the
        // frame to its image; the screen always lies within it.
        let mut bounds = (0, 0, width, height);
        for (image, x, y) in &pieces {
            if let Some((left, top, right, bottom)) = shown(image) {
                bounds.0 = bounds.0.min(x + left);
                bounds.1 = bounds.1.min(y + top);
                bounds.2 = bounds.2.max(x + right);
                bounds.3 = bounds.3.max(y + bottom);
            }
        }
        let size = |from: i64, to: i64| u32::try_from(to - from).map_err(|_| "the art is too large".to_owned());
        let mut drawn = RgbaImage::new(size(bounds.0, bounds.2)?, size(bounds.1, bounds.3)?);
        for (image, x, y) in &pieces {
            imageops::overlay(&mut drawn, image, x - bounds.0, y - bounds.1);
        }
        let screen = (size(bounds.0, 0)?, size(bounds.1, 0)?, layout.width, layout.height);
        Ok(Self { image: drawn, screen })
    }

    /// The art turned `turns` quarter turns counterclockwise, as the picture
    /// is, scaled so its screen matches a screen of `screen` pixels, which is
    /// the picture's, no larger, and the whole fits `area`: its PNG, its
    /// size, where the screen is, and the frame's pixels per pixel of the
    /// picture.
    pub fn render(&self, turns: u8, screen: (u32, u32), area: FrameArea) -> Result<Rendered, String> {
        let (image, placed) = self.turned(turns);
        let (_, _, shown_width, shown_height) = placed;
        let stretch = (
            f64::from(screen.0.max(1)) / f64::from(shown_width),
            f64::from(screen.1.max(1)) / f64::from(shown_height),
        );
        let (width, height) = (f64::from(image.width()) * stretch.0, f64::from(image.height()) * stretch.1);
        let fit = (f64::from(area.width) / width).min(f64::from(area.height) / height).min(1.0);
        let scale = (stretch.0 * fit, stretch.1 * fit);
        let size = ((width * fit).round().max(1.0) as u32, (height * fit).round().max(1.0) as u32);
        let resized = imageops::resize(&image, size.0, size.1, imageops::FilterType::CatmullRom);
        let mut png = std::io::Cursor::new(Vec::new());
        resized
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|error| format!("the frame did not encode: {error}"))?;
        Ok(Rendered {
            png: png.into_inner(),
            width: size.0,
            height: size.1,
            screen: PixelRect {
                x: f64::from(placed.0) * scale.0,
                y: f64::from(placed.1) * scale.1,
                width: f64::from(shown_width) * scale.0,
                height: f64::from(shown_height) * scale.1,
            },
            scale: fit,
        })
    }

    /// The art and its screen turned `turns` quarter turns counterclockwise.
    fn turned(&self, turns: u8) -> (RgbaImage, (u32, u32, u32, u32)) {
        let (x, y, width, height) = self.screen;
        let (art_width, art_height) = self.image.dimensions();
        match turns % 4 {
            0 => (self.image.clone(), self.screen),
            // A point (x, y) of the upright art lies at (y, width - x).
            1 => (imageops::rotate270(&self.image), (y, art_width - x - width, height, width)),
            2 => (imageops::rotate180(&self.image), (art_width - x - width, art_height - y - height, width, height)),
            _ => (imageops::rotate90(&self.image), (art_height - y - height, x, height, width)),
        }
    }
}

/// The art drawn for the view.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub screen: PixelRect,
    /// The frame's pixels per pixel of the picture.
    pub scale: f64,
}

/// The bounds of what `image` shows, left, top, right and bottom, or none
/// when it is all clear.
fn shown(image: &RgbaImage) -> Option<(i64, i64, i64, i64)> {
    let mut bounds: Option<(i64, i64, i64, i64)> = None;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] == 0 {
            continue;
        }
        let (x, y) = (i64::from(x), i64::from(y));
        bounds = Some(match bounds {
            None => (x, y, x + 1, y + 1),
            Some((left, top, right, bottom)) => (left.min(x), top.min(y), right.max(x + 1), bottom.max(y + 1)),
        });
    }
    bounds
}
