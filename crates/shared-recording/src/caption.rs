//! What a drawn export writes over the picture besides the inputs: the
//! note that shows at the time, as a video's subtitle, and the badge of a
//! wait played faster.
//!
//! ```text
//! +--------------------------------------------+
//! |                                  [▶▶ 8×]   |
//! |                                            |
//! | ╭─────────────────────────────╮            |
//! | │ 2/5 Hover the chart, then   │   ╭──────╮ |
//! | │ flip a switch               │   │ ⌘ ,  │ |
//! | ╰─────────────────────────────╯   ╰──────╯ |
//! +--------------------------------------------+
//! ```
//!
//! A note is in any language, so it is drawn with the fonts of the system,
//! each loaded the first time a character needs it, the crate's own last; an
//! emoji in color from the pictures of the system's emoji font. A character
//! none of them has is drawn as a box, and said. A note is two lines at
//! most; a longer one is drawn smaller, then cut, and said. In a video its
//! size follows the frame's longer side, not the screen's points: a video is
//! watched shrunk to fit, and a note must read at that size. A sheet draws
//! it over each shrunk tile, at a size of its own.
//!
//! A note sits at the bottom left, no wider than [`WIDEST`] of the frame, and
//! the key capsule at the bottom right, both [`margin`] from the edges, so
//! the two share a row and never cover each other.

use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};

use ab_glyph::{Font as _, FontArc, GlyphId, PxScale, ScaleFont as _};
use tiny_skia::{FillRule, PathBuilder, PixmapMut, Transform};

use crate::overlay::{blend, paint, rounded};

/// The fonts a note is drawn with, the first that has a character drawing
/// it: the system's own for text of every language, and the crate's last.
#[cfg(target_os = "macos")]
const SYSTEM: &[&str] = &[
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
    "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    "/System/Library/Fonts/Apple Symbols.ttf",
];
/// The system's emoji font, which a pictograph is looked for in first.
#[cfg(target_os = "macos")]
const EMOJI: &[&str] = &["/System/Library/Fonts/Apple Color Emoji.ttc"];
#[cfg(target_os = "windows")]
const SYSTEM: &[&str] = &[
    "C:\\Windows\\Fonts\\msyh.ttc",
    "C:\\Windows\\Fonts\\segoeui.ttf",
    "C:\\Windows\\Fonts\\YuGothM.ttc",
    "C:\\Windows\\Fonts\\malgun.ttf",
    "C:\\Windows\\Fonts\\seguisym.ttf",
];
#[cfg(target_os = "windows")]
const EMOJI: &[&str] = &["C:\\Windows\\Fonts\\seguiemj.ttf"];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SYSTEM: &[&str] = &[
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const EMOJI: &[&str] = &[
    "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf",
];

/// How many of the frame's longer side's pixels one em of a note is: a
/// note's line on a 1280-pixel frame is 27 pixels high.
const EM_PER_SIDE: f32 = 1.0 / 48.0;
/// The smallest a note is drawn, in pixels, however small the frame.
const SMALLEST: f32 = 13.0;
/// A line's height, in ems.
const LINE: f32 = 1.3;
/// The room around a note's lines in its box, across and down, in ems.
const PAD: (f32, f32) = (0.6, 0.3);
/// How far a note's box is from the frame's bottom and left edges, in ems.
const BOTTOM: f32 = 1.0;
/// The share of the frame's width a note's box may take.
const WIDEST: f32 = 0.6;
/// How far the badge is from the frame's top right corner, in ems.
const CORNER: f32 = 0.8;
/// How much smaller a note too long for two lines may be drawn before it
/// is cut.
const SHRINK: [f32; 3] = [1.0, 0.86, 0.74];
/// How much faster a wait plays; the badge says it.
pub(crate) const FAST: u64 = 8;

/// A note's box and the badge: black, a little transparent, as a video's
/// subtitle.
const BOX: ([u8; 3], f32) = ([0, 0, 0], 0.62);
const TEXT: [u8; 3] = [255, 255, 255];

/// A note laid out for one width of frame.
struct Layout {
    lines: Vec<String>,
    em: f32,
}

/// The notes of an export, with the fonts they loaded, and what they could
/// not show.
pub(crate) struct Caption {
    fonts: Vec<FontArc>,
    /// The system's fonts not loaded yet, for text and for emoji.
    unloaded: std::slice::Iter<'static, &'static str>,
    unloaded_emoji: std::slice::Iter<'static, &'static str>,
    /// Whether the crate's own font, the last, is loaded.
    own: bool,
    /// Which font draws each character met, none for one no font has.
    chosen: HashMap<char, Option<usize>>,
    layouts: HashMap<(String, u32, u32), Layout>,
    missing: BTreeSet<char>,
    /// What the notes could not show as set besides missing characters, a
    /// sentence each.
    said: BTreeSet<String>,
}

impl Caption {
    pub(crate) fn new() -> Self {
        Self {
            fonts: Vec::new(),
            unloaded: SYSTEM.iter(),
            unloaded_emoji: EMOJI.iter(),
            own: false,
            chosen: HashMap::new(),
            layouts: HashMap::new(),
            missing: BTreeSet::new(),
            said: BTreeSet::new(),
        }
    }


    /// Draws `note`, with the em `em`, at the bottom of the frame `pixmap`,
    /// in the middle; `about` names it in what could not be shown, such as
    /// `at 3.200 s of r1`.
    pub(crate) fn draw_note(&mut self, pixmap: &mut PixmapMut<'_>, em: f32, note: &str, about: &str) {
        let (width, height) = (pixmap.width() as f32, pixmap.height() as f32);
        let room = width * WIDEST - 2.0 * PAD.0 * em;
        let key = (note.to_owned(), pixmap.width(), em.to_bits());
        if !self.layouts.contains_key(&key) {
            let layout = self.layout(note, em, room, about);
            self.layouts.insert(key.clone(), layout);
        }
        let Layout { lines, em: size } = &self.layouts[&key];
        let (lines, size) = (lines.clone(), *size);
        let widest = lines.iter().map(|line| self.width(line, size)).fold(0.0, f32::max);
        let box_width = widest + 2.0 * PAD.0 * em;
        let box_height = lines.len() as f32 * LINE * size + 2.0 * PAD.1 * em;
        let (left, top) = (BOTTOM * em, height - BOTTOM * em - box_height);
        if let Some(path) = rounded(left, top, box_width, box_height, 0.35 * em) {
            pixmap.fill_path(&path, &paint(BOX.0, BOX.1), FillRule::Winding, Transform::identity(), None);
        }
        for (index, line) in lines.iter().enumerate() {
            self.text(pixmap, line, left + PAD.0 * em, top + PAD.1 * em + index as f32 * LINE * size, size, TEXT);
        }
    }

    /// Draws the badge of a wait played faster, with the em `em`, at the
    /// frame's top right corner.
    pub(crate) fn draw_badge(&mut self, pixmap: &mut PixmapMut<'_>, em: f32) {
        let label = format!("{FAST}×");
        let size = 0.9 * em;
        let width = 1.75 * em + self.width(&label, size) + 0.6 * em;
        let height = LINE * em;
        let (x, y) = (pixmap.width() as f32 - CORNER * em - width, CORNER * em);
        if let Some(capsule) = rounded(x, y, width, height, height / 2.0) {
            pixmap.fill_path(&capsule, &paint(BOX.0, BOX.1), FillRule::Winding, Transform::identity(), None);
        }
        let (middle, half) = (y + height / 2.0, 0.3 * em);
        let mut arrows = PathBuilder::new();
        for step in 0..2 {
            let left = x + 0.55 * em + step as f32 * 0.5 * em;
            arrows.move_to(left, middle - half);
            arrows.line_to(left + 0.52 * em, middle);
            arrows.line_to(left, middle + half);
            arrows.close();
        }
        if let Some(arrows) = arrows.finish() {
            pixmap.fill_path(&arrows, &paint(TEXT, 1.0), FillRule::Winding, Transform::identity(), None);
        }
        self.text(pixmap, &label, x + 1.75 * em, y + (height - LINE * size) / 2.0, size, TEXT);
    }

    /// How wide `text` is, `size` pixels an em.
    pub(crate) fn measure(&mut self, text: &str, size: f32) -> f32 {
        self.width(text, size)
    }

    /// How high a line of `size` pixels an em is.
    pub(crate) fn line(size: f32) -> f32 {
        LINE * size
    }

    /// Draws `text` in white, `size` pixels an em, its line's top left at
    /// `x`, `y`.
    pub(crate) fn label(&mut self, pixmap: &mut PixmapMut<'_>, text: &str, x: f32, y: f32, size: f32) {
        self.text(pixmap, text, x, y, size, TEXT);
    }

    /// What the notes could not show as they were set, in sentences.
    pub(crate) fn unshown(&self) -> Vec<String> {
        let mut said = Vec::new();
        if !self.missing.is_empty() {
            let characters = self.missing.iter().map(|character| format!("{character:?}")).collect::<Vec<_>>().join(", ");
            said.push(format!("No font on this Host has {characters}, drawn as boxes in the notes"));
        }
        said.extend(self.said.iter().cloned());
        said
    }

    /// `note` in lines no wider than `room`: as large as two lines hold it,
    /// cut at the end of the second at the smallest.
    fn layout(&mut self, note: &str, em: f32, room: f32, about: &str) -> Layout {
        // Characters are drawn one by one, each as it stands alone, which
        // is not how every script is written.
        if note.chars().any(joined) {
            self.said.insert(format!(
                "The note {about} is drawn letter by letter: a script that joins its letters or runs right to left, and an emoji made of several, are not drawn as written"
            ));
        }
        for share in SHRINK {
            let lines = self.wrap(note, em * share, room);
            if lines.len() <= 2 {
                return Layout { lines, em: em * share };
            }
        }
        let size = em * SHRINK[SHRINK.len() - 1];
        let mut lines = self.wrap(note, size, room);
        lines.truncate(2);
        let last = lines.last_mut().expect("a note too long has two lines");
        while !last.is_empty() && self.width(&format!("{last}…"), size) > room {
            last.pop();
        }
        last.push('…');
        self.said.insert(format!("The note {about} is longer than two lines, and is cut"));
        Layout { lines, em: size }
    }

    /// `text` broken into lines no wider than `room`: at spaces, around
    /// characters of scripts written without them, and within a word only
    /// when the word alone is wider.
    fn wrap(&mut self, text: &str, size: f32, room: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.lines() {
            let mut line = String::new();
            for word in words(paragraph) {
                let joined = if line.is_empty() { word.trim_start().to_owned() } else { format!("{line}{word}") };
                if self.width(&joined, size) <= room {
                    line = joined;
                    continue;
                }
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                for character in word.trim_start().chars() {
                    let longer = format!("{line}{character}");
                    if !line.is_empty() && self.width(&longer, size) > room {
                        lines.push(std::mem::take(&mut line));
                        line.push(character);
                    } else {
                        line = longer;
                    }
                }
            }
            lines.push(line);
        }
        lines
    }

    fn width(&mut self, text: &str, size: f32) -> f32 {
        text.chars().map(|character| self.advance(character, size)).sum()
    }

    fn advance(&mut self, character: char, size: f32) -> f32 {
        if ignorable(character) {
            return 0.0;
        }
        match self.font(character) {
            Some(index) => {
                let font = &self.fonts[index];
                let scaled = font.as_scaled(scale(font, size));
                scaled.h_advance(font.glyph_id(character))
            }
            None => box_width(size),
        }
    }

    /// Which font draws `character`, loading the system's next fonts until
    /// one has it; none when none does.
    fn font(&mut self, character: char) -> Option<usize> {
        if let Some(chosen) = self.chosen.get(&character) {
            return *chosen;
        }
        let has = |font: &FontArc| font.glyph_id(character) != GlyphId(0);
        let mut found = self.fonts.iter().position(has);
        while found.is_none() {
            let Some(font) = self.load_next(pictograph(character)) else {
                break;
            };
            self.fonts.push(font);
            found = has(self.fonts.last().expect("a font was loaded")).then(|| self.fonts.len() - 1);
        }
        if found.is_none() && !character.is_whitespace() && !character.is_control() {
            self.missing.insert(character);
        }
        self.chosen.insert(character, found);
        found
    }

    /// The next of the system's fonts there is, its emoji font first for a
    /// pictograph and last for another character, and then the crate's own:
    /// an emoji font is the largest by far.
    fn load_next(&mut self, pictograph: bool) -> Option<FontArc> {
        let (first, then) = match pictograph {
            true => (&mut self.unloaded_emoji, &mut self.unloaded),
            false => (&mut self.unloaded, &mut self.unloaded_emoji),
        };
        if let Some(font) = first.chain(then).find_map(|path| load(Path::new(path))) {
            return Some(font);
        }
        if std::mem::replace(&mut self.own, true) {
            return None;
        }
        Some(FontArc::try_from_slice(crate::overlay::FONT).expect("the crate's font is a font"))
    }

    /// Draws `text` with its line's top at `y`, each character in the font
    /// that has it, a box for one no font has.
    fn text(&mut self, pixmap: &mut PixmapMut<'_>, text: &str, x: f32, y: f32, size: f32, color: [u8; 3]) {
        let ascent = match self.font('A').or_else(|| text.chars().find_map(|character| self.font(character))) {
            Some(index) => self.fonts[index].as_scaled(scale(&self.fonts[index], size)).ascent(),
            None => size * 0.8,
        };
        let baseline = y + (LINE * size - size * 1.2) / 2.0 + ascent;
        let mut caret = x;
        for character in text.chars().filter(|character| !ignorable(*character)) {
            match self.font(character) {
                Some(index) => {
                    let font = self.fonts[index].clone();
                    let px = scale(&font, size);
                    let id = font.glyph_id(character);
                    let glyph = id.with_scale_and_position(px, ab_glyph::point(caret, baseline));
                    let advance = font.as_scaled(px).h_advance(id);
                    // A color font's emoji has an empty outline beside its
                    // picture.
                    if !draw_picture(pixmap, &font, id, caret, baseline, size)
                        && let Some(outline) = font.outline_glyph(glyph)
                    {
                        blend(pixmap, &outline, color, 1.0);
                    }
                    caret += advance;
                }
                None => {
                    let width = box_width(size);
                    if let Some(path) = rounded(caret + 0.1 * size, baseline - 0.72 * size, width - 0.2 * size, 0.78 * size, 0.0) {
                        let stroke = tiny_skia::Stroke {
                            width: (size / 14.0).max(1.0),
                            ..tiny_skia::Stroke::default()
                        };
                        pixmap.stroke_path(&path, &paint(color, 0.9), &stroke, Transform::identity(), None);
                    }
                    caret += width;
                }
            }
        }
    }
}

/// Whether `character` is of a script whose letters change shape with
/// their neighbors or run right to left, such as Arabic, Hebrew or the
/// scripts of India, or joins emoji into one, which a note drawn character
/// by character does not show as written.
fn joined(character: char) -> bool {
    matches!(u32::from(character), 0x0590..=0x08FF | 0x0900..=0x0DFF | 0x0F00..=0x0FFF | 0x1800..=0x18AF | 0x200D | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

/// Whether `character` is a pictograph, which an emoji font draws.
fn pictograph(character: char) -> bool {
    matches!(u32::from(character), 0x2300..=0x23FF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x1F000..=0x1FAFF)
}

/// Whether `character` takes no room and draws nothing, as a variation
/// selector or a joiner of an emoji does.
fn ignorable(character: char) -> bool {
    matches!(u32::from(character), 0x200B..=0x200F | 0x2060..=0x2064 | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF)
}

/// Draws the picture a color font gives for `id`, such as an emoji, an em
/// high on the line of `baseline` at `x`; whether it has one.
fn draw_picture(pixmap: &mut PixmapMut<'_>, font: &FontArc, id: GlyphId, x: f32, baseline: f32, size: f32) -> bool {
    let Some(picture) = font.glyph_raster_image2(id, size.ceil() as u16) else {
        return false;
    };
    if !matches!(picture.format, ab_glyph::GlyphImageFormat::Png) {
        return false;
    }
    let Ok(decoded) = image::load_from_memory_with_format(picture.data, image::ImageFormat::Png) else {
        return false;
    };
    let decoded = decoded.to_rgba8();
    let share = size / f32::from(picture.pixels_per_em.max(1));
    let (width, height) = (
        ((decoded.width() as f32 * share).round() as u32).max(1),
        ((decoded.height() as f32 * share).round() as u32).max(1),
    );
    let scaled = image::imageops::resize(&decoded, width, height, image::imageops::FilterType::Triangle);
    let (left, top) = (x.round() as i32, (baseline - 0.88 * size).round() as i32);
    let (canvas_width, canvas_height) = (pixmap.width() as i32, pixmap.height() as i32);
    let pixels = pixmap.data_mut();
    for (px, py, pixel) in scaled.enumerate_pixels() {
        let (cx, cy) = (left + px as i32, top + py as i32);
        if cx < 0 || cy < 0 || cx >= canvas_width || cy >= canvas_height {
            continue;
        }
        let a = f32::from(pixel[3]) / 255.0;
        let at = (cy as usize * canvas_width as usize + cx as usize) * 4;
        for channel in 0..3 {
            let under = f32::from(pixels[at + channel]);
            pixels[at + channel] = (under + (f32::from(pixel[channel]) - under) * a).round() as u8;
        }
    }
    true
}

/// How far a note's box and the key capsule are from the frame's edges, with
/// a note's em `em`.
pub(crate) fn margin(em: f32) -> f32 {
    BOTTOM * em
}

/// The em of a note on a video's frame of `width` by `height`.
pub(crate) fn em(width: u32, height: u32) -> f32 {
    (width.max(height) as f32 * EM_PER_SIDE).max(SMALLEST)
}

fn box_width(size: f32) -> f32 {
    0.62 * size
}

/// The scale that makes one em of `font` `size` pixels, whatever its units.
fn scale(font: &FontArc, size: f32) -> PxScale {
    font.pt_to_px_scale(size * 72.0 / 96.0).unwrap_or(PxScale::from(size))
}

fn load(path: &Path) -> Option<FontArc> {
    let bytes = std::fs::read(path).ok()?;
    match ab_glyph::FontVec::try_from_vec_and_index(bytes, 0) {
        Ok(font) => Some(FontArc::new(font)),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "a system font did not load, so notes are drawn without it");
            None
        }
    }
}

/// `text` as the pieces a line may break between: a word with the spaces
/// before it, or one character of a script written without spaces.
fn words(text: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    for character in text.chars() {
        if unspaced(character) {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            words.push(character.to_string());
            in_word = false;
        } else if character.is_whitespace() {
            if in_word {
                words.push(std::mem::take(&mut current));
                in_word = false;
            }
            current.push(character);
        } else {
            current.push(character);
            in_word = true;
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Whether `character` is of a script written without spaces between its
/// words, such as Chinese or Japanese, or is their punctuation.
fn unspaced(character: char) -> bool {
    matches!(u32::from(character), 0x2E80..=0x9FFF | 0xA960..=0xA97F | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

