//! Android Studio's device art (`mobile.md` § The live view): for each
//! device, such as `pixel_10`, a folder in Android Studio's installation
//! holding the frame's image, the screen's mask and a `layout` in the
//! emulator's skin format that places the screen in the frame. It is drawn
//! as Android Studio's emulator window draws it (`SkinDefinition` in its
//! `adt-ui`): the frame's image around the screen and never over it, the
//! mask over the screen's corners, the device's buttons, the whole turned
//! with the picture. Demi reads the art from the Host's installation and
//! carries none.

use std::{
    cmp::Reverse,
    path::{Path, PathBuf},
};

use image::{RgbaImage, imageops};
use serde::Deserialize;

use crate::driver::{FrameArea, PixelRect};

/// Where an installation keeps its device art.
const ART: [&str; 4] = ["plugins", "android", "resources", "device-art-resources"];

/// The operating systems whose Android Studio installations differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum System {
    MacOs,
    Windows,
    Linux,
}

impl System {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// The places an installation is looked for: the file system's root, the
/// user's home, and Windows' program folders.
#[derive(Debug, Clone, Default)]
pub struct Places {
    pub root: PathBuf,
    pub home: Option<PathBuf>,
    /// `%ProgramFiles%`.
    pub program_files: Option<PathBuf>,
    /// `%LOCALAPPDATA%`.
    pub local_app_data: Option<PathBuf>,
}

impl Places {
    /// This Host's.
    pub fn current() -> Self {
        let variable = |name: &str| std::env::var_os(name).map(PathBuf::from);
        Self {
            root: PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }),
            home: variable(if cfg!(windows) { "USERPROFILE" } else { "HOME" }),
            program_files: variable("ProgramFiles"),
            local_app_data: variable("LOCALAPPDATA"),
        }
    }
}

/// The entries of `directory` whose names start with `prefix`, the shortest
/// name first, so that of installations whose releases do not read, a
/// release (`Android Studio.app`) comes before a preview of it
/// (`Android Studio Preview.app`).
fn named(directory: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(prefix)))
        .collect();
    found.sort_by_key(|path| (path.as_os_str().len(), path.clone()));
    found
}

/// What an installation's `product-info.json` says of its release.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProductInfo {
    /// Such as `262.9437.185.2621.16467767`.
    build_number: String,
    /// Such as `AndroidStudio2026.2.1`, or `AndroidStudioPreview2026.3.1`
    /// for a preview.
    data_directory_name: String,
}

/// An installation's rank: a stable release before a preview, then the
/// newer build first; one whose release does not read last.
fn rank(installation: &Path) -> (bool, bool, Reverse<Vec<u64>>) {
    // A Mac's app keeps it in its resources, the others at their top.
    let info = [installation.join("Resources").join("product-info.json"), installation.join("product-info.json")]
        .iter()
        .find_map(|file| std::fs::read(file).ok())
        .and_then(|bytes| serde_json::from_slice::<ProductInfo>(&bytes).ok());
    match info {
        Some(info) => {
            let preview = info.data_directory_name.starts_with("AndroidStudioPreview");
            let build = info.build_number.split('.').map_while(|part| part.parse().ok()).collect();
            (false, preview, Reverse(build))
        }
        None => (true, false, Reverse(Vec::new())),
    }
}

/// The installations of Android Studio on `system`, in the order they are
/// tried: where its installer, the user, or JetBrains Toolbox puts it,
/// ranked by their releases, a stable release before a preview wherever
/// each is, then the newer before the older.
pub fn installations(system: System, places: &Places) -> Vec<PathBuf> {
    let mut found = places_of(system, places);
    // Stable, so installations alike keep the order of their places.
    found.sort_by_cached_key(|installation| rank(installation));
    found
}

/// The installations of Android Studio on `system`, in the order of their
/// places.
fn places_of(system: System, places: &Places) -> Vec<PathBuf> {
    let mut found = Vec::new();
    match system {
        System::MacOs => {
            let mut apps = vec![places.root.join("Applications")];
            apps.extend(places.home.iter().map(|home| home.join("Applications")));
            for directory in apps {
                found.extend(
                    named(&directory, "Android Studio")
                        .into_iter()
                        .filter(|app| app.extension().is_some_and(|extension| extension == "app"))
                        .map(|app| app.join("Contents")),
                );
            }
        }
        System::Windows => {
            if let Some(program_files) = &places.program_files {
                found.extend(named(&program_files.join("Android"), "Android Studio"));
            }
            if let Some(local) = &places.local_app_data {
                found.extend(named(&local.join("Programs"), "Android Studio"));
            }
        }
        System::Linux => {
            for directory in [places.root.join("opt"), places.root.join("usr").join("local")] {
                found.extend(named(&directory, "android-studio"));
            }
            found.push(places.root.join("snap").join("android-studio").join("current").join("android-studio"));
            if let Some(home) = &places.home {
                found.extend(named(home, "android-studio"));
                found.extend(named(&home.join(".local").join("share").join("JetBrains").join("Toolbox").join("apps"), "android-studio"));
            }
        }
    }
    found
}

/// The art for `device` in the first of `installations` that has it.
pub fn find(installations: &[PathBuf], device: &str) -> Option<PathBuf> {
    // The device's name is a folder's, never a path.
    if device.is_empty() || device.contains(['/', '\\']) || device.starts_with('.') {
        return None;
    }
    installations
        .iter()
        .map(|installation| ART.iter().fold(installation.clone(), |path, part| path.join(part)).join(device))
        .find(|folder| folder.join("layout").is_file())
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
