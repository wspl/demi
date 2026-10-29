//! A sheet: one PNG of a whole recording, or of several, so that a model,
//! which reads no video, sees all of it at once.
//!
//! ```text
//! r1 0.000 s     r1 1.500 s     r1 3.000 s     r1 4.500 s
//! +----------+   +----------+   +----------+   +----------+
//! |  frame   |   |  frame   |   |  frame   |   |  frame   |
//! | [note]   |   | [note]   |   | [note]   |   | [note]   |
//! +----------+   +----------+   +----------+   +----------+
//! r1 6.000 s     r1 6.214 s     r2 0.000 s     ...
//! ```
//!
//! A tile is the screen at a time of a recording, drawn as a drawn export
//! draws it, shrunk, with its note drawn over the shrunk tile at a size that
//! reads there. The times are every `--every` from the recording's
//! start, and its end, so a sheet never leaves out how a recording ended.

use std::path::Path;

use demi_shared_openh264::Load;
use demi_shared_recording_protocol::text::seconds;
use image::{ImageEncoder as _, RgbaImage, codecs::png::PngEncoder, imageops};

use crate::{
    Error,
    caption::{self, Caption},
    overlay::Overlay,
    video::{Painter, Part},
};

/// How many tiles a sheet has by default, at most: as many as an image a
/// model reads shows legibly.
const TILES: u64 = 24;
/// The most tiles a sheet has with any `--every`: a sheet's pixels are held
/// whole while it is made, and its tiles shrink past reading.
const MOST: usize = 120;
/// How long a tile's longer side is, in pixels.
const TILE: u32 = 360;
/// The em of a note below a tile, in pixels.
const NOTE_EM: f32 = 13.0;
/// The default time between tiles is a whole number of these.
const ROUNDED_US: u64 = 100_000;
const GAP: u32 = 8;
/// A label's em, in pixels, and its margin above and below.
const LABEL_SIZE: f32 = 15.0;
const LABEL_MARGIN: u32 = 4;
const BACKGROUND: [u8; 4] = [32, 32, 32, 255];

/// What a sheet asks for.
pub(crate) struct Sheet<'a> {
    pub parts: &'a [Part],
    /// The time between tiles, in microseconds; by default the one that
    /// gives at most [`TILES`].
    pub every: Option<u64>,
    /// Whether the tiles leave the inputs and the notes off.
    pub plain: bool,
    pub load: &'a Load,
}

/// Writes the sheet to `path`, and says what of the notes it could not
/// show as set.
pub(crate) fn write(path: &Path, sheet: &Sheet<'_>) -> Result<Vec<String>, Error> {
    let every = sheet.every.unwrap_or_else(|| default_every(sheet.parts));
    let times: Vec<Vec<u64>> = sheet.parts.iter().map(|part| times(part.ran, every)).collect();
    let count: usize = times.iter().map(Vec::len).sum();
    if count > MOST {
        return Err(Error::BadRequest(format!(
            "a sheet every {} s would have {count} tiles, more than the {MOST} it holds; give a longer --every",
            seconds(every)
        )));
    }
    let mut caption = Caption::new();
    let mut tiles: Vec<(String, RgbaImage)> = Vec::with_capacity(count);
    let mut canvas = Vec::new();
    for (part, times) in sheet.parts.iter().zip(&times) {
        let overlay = Overlay::new(&part.contents.inputs);
        let drawn = !sheet.plain && !overlay.is_empty();
        let mut painter = Painter::new(part, drawn.then_some(&overlay), sheet.load)?;
        let captioned = !sheet.plain && part.has_notes();
        for &t in times {
            painter.advance(t)?;
            let Some((width, height)) = painter.size() else {
                continue;
            };
            let shrink = (f64::from(TILE) / f64::from(width.max(height))).min(1.0);
            // The key capsule in the row of the note the shrunk tile shows.
            painter.paint(t, caption::margin(NOTE_EM) / shrink as f32, &mut canvas);
            let picture = RgbaImage::from_raw(width, height, std::mem::take(&mut canvas))
                .expect("a painted picture has its size's pixels");
            let mut tile = match shrink < 1.0 {
                true => imageops::resize(
                    &picture,
                    ((f64::from(width) * shrink).round() as u32).max(1),
                    ((f64::from(height) * shrink).round() as u32).max(1),
                    imageops::FilterType::Triangle,
                ),
                false => picture,
            };
            let (tile_width, tile_height) = tile.dimensions();
            if captioned && let Some(mut pixmap) = tiny_skia::PixmapMut::from_bytes(&mut tile, tile_width, tile_height) {
                part.caption(&mut caption, t, &mut pixmap, NOTE_EM);
            }
            tiles.push((format!("{} {} s", part.id, seconds(t)), tile));
        }
    }
    let png = laid_out(&tiles, &mut caption)?;
    std::fs::write(path, png).map_err(|error| Error::io(path, &error))?;
    Ok(caption.unshown())
}

/// The time between tiles that gives at most [`TILES`] over `parts`, each
/// with a tile at its start and its end, a whole number of tenths of a
/// second.
fn default_every(parts: &[Part]) -> u64 {
    let total: u64 = parts.iter().map(|part| part.ran).sum();
    let longest = parts.iter().map(|part| part.ran).max().unwrap_or(0);
    // Each part may add a tile at its start and one at its end to its
    // share of the rest.
    let shares = TILES.saturating_sub(2 * parts.len() as u64);
    if shares == 0 {
        return longest.max(ROUNDED_US);
    }
    total.div_ceil(shares).div_ceil(ROUNDED_US).max(1) * ROUNDED_US
}

/// The tiles' times of a recording that ran `ran`: every `every` from its
/// start, and its end.
fn times(ran: u64, every: u64) -> Vec<u64> {
    let mut times: Vec<u64> = (0..).map(|step| step * every).take_while(|t| *t < ran).collect();
    times.push(ran);
    times.dedup();
    times
}

/// The tiles in rows, each under its label, as a PNG.
fn laid_out(tiles: &[(String, RgbaImage)], caption: &mut Caption) -> Result<Vec<u8>, Error> {
    let cell_width = tiles
        .iter()
        .map(|(text, tile)| tile.width().max(caption.measure(text, LABEL_SIZE).ceil() as u32))
        .max()
        .unwrap_or(1);
    let label = Caption::line(LABEL_SIZE).ceil() as u32 + 2 * LABEL_MARGIN;
    // The tiles' height on average, which the number of columns is chosen by.
    let cell_height = tiles.iter().map(|(_, tile)| tile.height()).sum::<u32>() / tiles.len().max(1) as u32 + label;
    // As many columns as make the sheet about half again as wide as high.
    let count = tiles.len().max(1);
    let columns = ((count as f64 * f64::from(cell_height) / f64::from(cell_width) * 1.5).sqrt().round() as usize).clamp(1, count);
    // Each row as high as its highest tile, so that a row of landscape tiles
    // leaves no room for a portrait one it does not hold.
    let tops: Vec<u32> = tiles
        .chunks(columns)
        .scan(GAP, |top, row| {
            let this = *top;
            *top += row.iter().map(|(_, tile)| tile.height()).max().unwrap_or(0) + label + GAP;
            Some(this)
        })
        .collect();
    let width = GAP + columns as u32 * (cell_width + GAP);
    let last = tiles.chunks(columns).last().map_or(0, |row| row.iter().map(|(_, tile)| tile.height()).max().unwrap_or(0));
    let height = tops.last().copied().unwrap_or(GAP) + last + label + GAP;
    let mut sheet = RgbaImage::from_pixel(width, height, image::Rgba(BACKGROUND));
    for (index, (text, tile)) in tiles.iter().enumerate() {
        let (column, row) = ((index % columns) as u32, index / columns);
        let (x, y) = (GAP + column * (cell_width + GAP), tops[row]);
        imageops::replace(&mut sheet, tile, i64::from(x + (cell_width - tile.width()) / 2), i64::from(y + label));
        if let Some(mut pixmap) = tiny_skia::PixmapMut::from_bytes(&mut sheet, width, height) {
            caption.label(&mut pixmap, text, x as f32, (y + LABEL_MARGIN) as f32, LABEL_SIZE);
        }
    }
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(sheet.as_raw(), width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| Error::Failed(format!("the sheet did not encode: {error}")))?;
    Ok(png)
}
