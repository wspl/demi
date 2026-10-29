//! What a drawn export draws on the frames: the inputs a recording's source
//! took, at the time each was taken (`inputs.jsonl`).
//!
//! ```text
//!   ↖ the pointer, easing to each place     ◯ a press: a ring that grows
//!   ⌄⌄ a wheel turn, a chevron a notch      ● a finger, its trail behind it
//!            ╭──────────╮
//!            │ ⌘ ,      │  a key or a shortcut, or ⌨ 12 for text typed
//!            ╰──────────╯
//! ```
//!
//! Sizes are in points, each input's `scale` frame pixels to the point, so a
//! drawing is as large on a Retina capture as on another. What moves, eases
//! or fades does so over a fixed time after its input, so every frame of a
//! time draws the same, and [`Overlay::moving`] says when the export must
//! code frames between the recording's own to show it move.

use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont as _};
use demi_shared_recording_protocol::{Input, InputPhase, InputRow, InputButton};
use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, PixmapMut, Stroke, Transform};

/// The one font a drawn export uses: the glyphs of key names and of the
/// symbols macOS names modifiers by, cut from DejaVu Sans Bold
/// (`LICENSE-DejaVu`).
pub(crate) const FONT: &[u8] = include_bytes!("../fonts/DejaVuSans-Bold-keys.ttf");

/// How long the pointer takes to reach a place it jumps to: an agent's
/// pointer jumps, and the eye loses one it cannot follow. It arrives when
/// the input did, never later.
const GLIDE_US: u64 = 150_000;
/// How long a press's ring grows and fades.
const RIPPLE_US: u64 = 400_000;
/// How long a wheel turn's chevrons fade.
const WHEEL_US: u64 = 600_000;
/// How long a key's capsule stays, unless the next one replaces it, and how
/// long text typed in a row adds to one capsule.
const CAPSULE_US: u64 = 1_500_000;
/// How long a lifted finger and its trail fade.
const LIFT_US: u64 = 300_000;
/// How often frames are coded while something moves: 30 a second.
pub(crate) const STEP_US: u64 = 33_333;

const BLUE: [u8; 3] = [10, 132, 255];
const ORANGE: [u8; 3] = [255, 159, 10];
const GREY: [u8; 3] = [142, 142, 147];

/// A place the pointer was put, and when.
#[derive(Clone, Copy)]
struct Place {
    t: u64,
    x: f32,
    y: f32,
    scale: f32,
}

/// A press of a mouse button.
struct Press {
    t: u64,
    x: f32,
    y: f32,
    scale: f32,
    button: InputButton,
}

/// A press held from `t` to `until`, through the places of a drag.
struct Held {
    t: u64,
    until: u64,
}

/// A wheel turn.
struct Wheel {
    t: u64,
    dx: f32,
    dy: f32,
}

/// A capsule at the bottom of the frame, from `t` to `until`.
struct Capsule {
    t: u64,
    until: u64,
    scale: f32,
    shows: Shows,
}

/// What a capsule says.
enum Shows {
    Keys(String),
    /// Text typed in a row: when each run of characters was typed, so that
    /// the count grows as the typing does.
    Typed(Vec<(u64, u64)>),
}

/// A finger on the screen from `down` to `up`, through `path`.
struct Finger {
    down: u64,
    up: u64,
    path: Vec<Place>,
}

/// The inputs of a recording, ready to draw at any time.
pub(crate) struct Overlay {
    font: FontRef<'static>,
    places: Vec<Place>,
    presses: Vec<Press>,
    held: Vec<Held>,
    wheels: Vec<Wheel>,
    capsules: Vec<Capsule>,
    fingers: Vec<Finger>,
}

impl Overlay {
    pub(crate) fn new(rows: &[InputRow]) -> Self {
        let mut overlay = Self {
            font: FontRef::try_from_slice(FONT).expect("the crate's font is a font"),
            places: Vec::new(),
            presses: Vec::new(),
            held: Vec::new(),
            wheels: Vec::new(),
            capsules: Vec::new(),
            fingers: Vec::new(),
        };
        let mut down: Option<u64> = None;
        let mut finger: Option<Finger> = None;
        for row in rows {
            let t = row.time_us;
            match &row.input {
                Input::Pointer { x, y, scale: s, phase, button, .. } => {
                    overlay.places.push(Place { t, x: *x as f32, y: *y as f32, scale: *s as f32 });
                    match phase {
                        InputPhase::Down => {
                            overlay.presses.push(Press {
                                t,
                                x: *x as f32,
                                y: *y as f32,
                                scale: *s as f32,
                                button: button.unwrap_or(InputButton::Left),
                            });
                            down.get_or_insert(t);
                        }
                        InputPhase::Up => {
                            if let Some(from) = down.take() {
                                overlay.held.push(Held { t: from, until: t });
                            }
                        }
                        InputPhase::Move => {}
                    }
                }
                Input::Wheel { x, y, scale: s, dx, dy, .. } => {
                    overlay.places.push(Place { t, x: *x as f32, y: *y as f32, scale: *s as f32 });
                    overlay.wheels.push(Wheel { t, dx: *dx as f32, dy: *dy as f32 });
                }
                Input::Touch { x, y, scale: s, phase, .. } => {
                    let place = Place { t, x: *x as f32, y: *y as f32, scale: *s as f32 };
                    match phase {
                        InputPhase::Down => {
                            if let Some(mut open) = finger.take() {
                                open.up = t;
                                overlay.fingers.push(open);
                            }
                            finger = Some(Finger { down: t, up: u64::MAX, path: vec![place] });
                        }
                        InputPhase::Move => {
                            if let Some(open) = finger.as_mut() {
                                open.path.push(place);
                            }
                        }
                        InputPhase::Up => {
                            if let Some(mut open) = finger.take() {
                                open.path.push(place);
                                open.up = t;
                                overlay.fingers.push(open);
                            }
                        }
                    }
                }
                Input::Keys { keys, scale, .. } => {
                    let named = keys.iter().map(|key| key_name(key)).collect::<Vec<_>>().join(" ");
                    overlay.capsule(t, *scale as f32, Shows::Keys(named));
                }
                Input::Button { name, scale, .. } => overlay.capsule(t, *scale as f32, Shows::Keys(button_name(name))),
                Input::Typed { characters, scale, .. } => {
                    // Text typed in a row is one capsule, whose count grows
                    // with each key.
                    match overlay.capsules.last_mut() {
                        Some(Capsule { until, shows: Shows::Typed(runs), .. }) if t < *until => {
                            runs.push((t, u64::from(*characters)));
                            *until = t + CAPSULE_US;
                        }
                        _ => overlay.capsule(t, *scale as f32, Shows::Typed(vec![(t, u64::from(*characters))])),
                    }
                }
                // A note is the band's, below the picture (`band.rs`).
                Input::Command { .. } | Input::Note { .. } => {}
            }
        }
        if let Some(from) = down {
            overlay.held.push(Held { t: from, until: u64::MAX });
        }
        if let Some(open) = finger {
            overlay.fingers.push(open);
        }
        overlay
    }

    /// Adds a capsule at `t`, which ends the one before.
    fn capsule(&mut self, t: u64, scale: f32, shows: Shows) {
        if let Some(last) = self.capsules.last_mut() {
            last.until = last.until.min(t);
        }
        self.capsules.push(Capsule {
            t,
            until: t + CAPSULE_US,
            scale,
            shows,
        });
    }

    /// Whether it draws nothing at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.places.is_empty() && self.capsules.is_empty() && self.fingers.is_empty()
    }

    /// The times something it draws starts, ends or moves between, in
    /// order: the export codes a frame at each, besides the recording's own.
    pub(crate) fn changes(&self, end: u64) -> Vec<u64> {
        let mut times = Vec::new();
        let span = |times: &mut Vec<u64>, from: u64, until: u64| {
            let until = until.min(end);
            let mut t = from;
            while t < until {
                times.push(t);
                t += STEP_US;
            }
            times.push(until);
        };
        for place in &self.places {
            span(&mut times, place.t.saturating_sub(GLIDE_US), place.t);
        }
        for press in &self.presses {
            span(&mut times, press.t, press.t + RIPPLE_US);
        }
        for wheel in &self.wheels {
            span(&mut times, wheel.t, wheel.t + WHEEL_US);
        }
        for capsule in &self.capsules {
            times.push(capsule.t);
            times.push(capsule.until.min(end));
            if let Shows::Typed(runs) = &capsule.shows {
                times.extend(runs.iter().map(|(t, _)| *t));
            }
        }
        for finger in &self.fingers {
            span(&mut times, finger.down, finger.up.min(end).saturating_add(LIFT_US));
        }
        for held in &self.held {
            times.push(held.t);
            times.push(held.until.min(end));
        }
        times.retain(|t| *t <= end);
        times.sort_unstable();
        times.dedup();
        times
    }

    /// Draws what shows at `t` on the opaque RGBA frame `rgba`, the key
    /// capsule in the bottom right corner, `margin` pixels from its edges.
    pub(crate) fn draw(&self, t: u64, rgba: &mut [u8], width: u32, height: u32, margin: f32) {
        let Some(mut pixmap) = PixmapMut::from_bytes(rgba, width, height) else {
            return;
        };
        for finger in &self.fingers {
            self.draw_finger(&mut pixmap, finger, t);
        }
        // One mouse makes one press at a time: a press ends the ring of the
        // one before, so presses in quick succession never read as several
        // fingers.
        let latest = self.presses.partition_point(|press| press.t <= t);
        let latest = latest.checked_sub(1).map(|index| &self.presses[index]);
        if let Some(press) = latest.filter(|press| t < press.t + RIPPLE_US) {
            self.draw_ripple(&mut pixmap, press, t);
        }
        if let Some(place) = self.pointer(t) {
            let held = self.held.iter().find(|held| held.t <= t && t < held.until);
            if let Some(held) = held {
                self.draw_drag(&mut pixmap, held, &place);
            }
            let pressed = held.is_some();
            if let Some(wheel) = self.wheels.iter().rev().find(|wheel| wheel.t <= t && t < wheel.t + WHEEL_US) {
                self.draw_wheel(&mut pixmap, &place, wheel, t);
            }
            // The pointer takes the button's color while it is held, and for
            // as long as the ring of a click shows.
            let fill = latest
                .filter(|press| pressed || t < press.t + RIPPLE_US)
                .map_or([0, 0, 0], |press| button_color(press.button));
            draw_arrow(&mut pixmap, &place, if pressed { 0.85 } else { 1.0 }, fill);
        }
        if let Some(capsule) = self.capsules.iter().rev().find(|capsule| capsule.t <= t && t < capsule.until) {
            let text = match &capsule.shows {
                Shows::Keys(named) => named.clone(),
                Shows::Typed(runs) => format!("⌨ {}", runs.iter().filter(|(at, _)| *at <= t).map(|(_, count)| count).sum::<u64>()),
            };
            self.draw_capsule(&mut pixmap, &text, capsule.scale, width, height, margin);
        }
    }

    /// Where the pointer is at `t`: at its last place, or on its way to the
    /// next one in the last [`GLIDE_US`] before it. Before its first place
    /// it waits there, as a mouse rests where it is next used; none for a
    /// recording that took no pointer.
    fn pointer(&self, t: u64) -> Option<Place> {
        let next = self.places.partition_point(|place| place.t <= t);
        let Some(from) = next.checked_sub(1).map(|index| self.places[index]) else {
            return self.places.first().copied();
        };
        let Some(coming) = self.places.get(next) else {
            return Some(from);
        };
        let start = coming.t.saturating_sub(GLIDE_US).max(from.t);
        if t < start {
            return Some(from);
        }
        let share = (t - start) as f32 / (coming.t - start).max(1) as f32;
        let eased = 1.0 - (1.0 - share).powi(3);
        Some(Place {
            t,
            x: from.x + (coming.x - from.x) * eased,
            y: from.y + (coming.y - from.y) * eased,
            scale: coming.scale,
        })
    }

    fn draw_ripple(&self, pixmap: &mut PixmapMut<'_>, press: &Press, t: u64) {
        let share = (t - press.t) as f32 / RIPPLE_US as f32;
        let radius = (10.0 + 26.0 * share) * press.scale;
        let alpha = 1.0 - share;
        let color = button_color(press.button);
        if let Some(circle) = PathBuilder::from_circle(press.x, press.y, radius) {
            let stroke = Stroke {
                width: 4.0 * press.scale,
                ..Stroke::default()
            };
            pixmap.stroke_path(&circle, &paint(color, alpha), &stroke, Transform::identity(), None);
        }
        let label = match press.button {
            InputButton::Left => return,
            InputButton::Right => "R",
            InputButton::Middle => "M",
        };
        let size = 15.0 * press.scale;
        let (x, y) = (press.x + 18.0 * press.scale, press.y + 26.0 * press.scale);
        let width = self.text_width(label, size) + 8.0 * press.scale;
        if let Some(badge) = rounded(x, y, width, size + 6.0 * press.scale, (size + 6.0 * press.scale) / 2.0) {
            pixmap.fill_path(&badge, &paint(color, alpha.max(0.0) / 0.9), FillRule::Winding, Transform::identity(), None);
        }
        self.text(pixmap, label, x + 4.0 * press.scale, y + 3.0 * press.scale, size, [255, 255, 255], alpha / 0.9);
    }

    fn draw_wheel(&self, pixmap: &mut PixmapMut<'_>, place: &Place, wheel: &Wheel, t: u64) {
        let alpha = 1.0 - (t - wheel.t) as f32 / WHEEL_US as f32;
        let (dx, dy) = (wheel.dx, wheel.dy);
        let vertical = dy.abs() >= dx.abs();
        let amount = if vertical { dy } else { dx };
        // A notch of a mouse wheel moves 100 pixels in Chrome. Past five
        // chevrons, their number says how many.
        let notches = ((amount.abs() / 100.0).round() as usize).max(1);
        let drawn = notches.min(5);
        let sign = if amount >= 0.0 { 1.0 } else { -1.0 };
        let s = place.scale;
        let mut path = PathBuilder::new();
        for notch in 0..drawn {
            let offset = (18.0 + 10.0 * notch as f32) * s;
            let (cx, cy) = if vertical {
                (place.x + 30.0 * s, place.y + 12.0 * s + sign * (offset - 18.0 * s))
            } else {
                (place.x + 12.0 * s + sign * offset, place.y + 30.0 * s)
            };
            let arm = 8.0 * s;
            if vertical {
                path.move_to(cx - arm, cy - sign * arm / 2.0);
                path.line_to(cx, cy + sign * arm / 2.0);
                path.line_to(cx + arm, cy - sign * arm / 2.0);
            } else {
                path.move_to(cx - sign * arm / 2.0, cy - arm);
                path.line_to(cx + sign * arm / 2.0, cy);
                path.line_to(cx - sign * arm / 2.0, cy + arm);
            }
        }
        if let Some(path) = path.finish() {
            let outline = Stroke {
                width: 7.0 * s,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            pixmap.stroke_path(&path, &paint([255, 255, 255], alpha), &outline, Transform::identity(), None);
            let line = Stroke {
                width: 3.5 * s,
                ..outline
            };
            pixmap.stroke_path(&path, &paint(BLUE, alpha), &line, Transform::identity(), None);
        }
        if notches > drawn {
            let size = 15.0 * s;
            let (x, y) = if vertical { (place.x + 42.0 * s, place.y + 12.0 * s) } else { (place.x + 12.0 * s, place.y + 42.0 * s) };
            self.text(pixmap, &format!("×{notches}"), x, y, size, BLUE, alpha);
        }
    }

    fn draw_finger(&self, pixmap: &mut PixmapMut<'_>, finger: &Finger, t: u64) {
        if t < finger.down || t >= finger.up.saturating_add(LIFT_US) {
            return;
        }
        let alpha = if t < finger.up {
            1.0
        } else {
            1.0 - (t - finger.up) as f32 / LIFT_US as f32
        };
        let at = finger_at(finger, t);
        let s = at.scale;
        // The trail of the places it went through, up to where it is.
        let mut trail = PathBuilder::new();
        let mut started = false;
        for place in finger.path.iter().take_while(|place| place.t <= t) {
            if started {
                trail.line_to(place.x, place.y);
            } else {
                trail.move_to(place.x, place.y);
                started = true;
            }
        }
        if started {
            trail.line_to(at.x, at.y);
            if let Some(trail) = trail.finish() {
                let stroke = Stroke {
                    width: 8.0 * s,
                    line_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..Stroke::default()
                };
                pixmap.stroke_path(&trail, &paint(GREY, 0.35 * alpha), &stroke, Transform::identity(), None);
            }
        }
        // A long press grows its disc.
        let held = t.min(finger.up).saturating_sub(finger.down);
        let radius = (20.0 + 8.0 * (held as f32 / 1_000_000.0).min(1.0)) * s;
        if let Some(disc) = PathBuilder::from_circle(at.x, at.y, radius) {
            pixmap.fill_path(&disc, &paint([60, 60, 67], 0.5 * alpha), FillRule::Winding, Transform::identity(), None);
            let stroke = Stroke {
                width: 2.5 * s,
                ..Stroke::default()
            };
            pixmap.stroke_path(&disc, &paint([255, 255, 255], 0.9 * alpha), &stroke, Transform::identity(), None);
        }
    }

    /// The line a held button draws behind the pointer, through the places
    /// it moved since the press.
    fn draw_drag(&self, pixmap: &mut PixmapMut<'_>, held: &Held, at: &Place) {
        let mut line = PathBuilder::new();
        let mut started = false;
        for place in self.places.iter().filter(|place| place.t >= held.t && place.t <= at.t) {
            if started {
                line.line_to(place.x, place.y);
            } else {
                line.move_to(place.x, place.y);
                started = true;
            }
        }
        if !started {
            return;
        }
        line.line_to(at.x, at.y);
        if let Some(line) = line.finish() {
            let stroke = Stroke {
                width: 4.0 * at.scale,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            pixmap.stroke_path(&line, &paint(BLUE, 0.45), &stroke, Transform::identity(), None);
        }
    }

    fn draw_capsule(&self, pixmap: &mut PixmapMut<'_>, text: &str, scale: f32, width: u32, height: u32, margin: f32) {
        let s = scale;
        let size = 22.0 * s;
        let pad = 18.0 * s;
        let box_height = size + 18.0 * s;
        let box_width = self.text_width(text, size) + 2.0 * pad;
        let (x, y) = (width as f32 - margin - box_width, height as f32 - margin - box_height);
        if let Some(path) = rounded(x, y, box_width, box_height, box_height / 2.0) {
            pixmap.fill_path(&path, &paint([28, 28, 30], 0.85), FillRule::Winding, Transform::identity(), None);
        }
        self.text(pixmap, text, x + pad, y + 9.0 * s, size, [255, 255, 255], 1.0);
    }

    fn text_width(&self, text: &str, size: f32) -> f32 {
        let font = self.font.as_scaled(PxScale::from(size));
        text.chars().map(|character| font.h_advance(font.glyph_id(character))).sum()
    }

    /// Draws `text` with its top at `y`, by the coverage of each glyph's
    /// outline, blended over the frame.
    fn text(&self, pixmap: &mut PixmapMut<'_>, text: &str, x: f32, y: f32, size: f32, color: [u8; 3], alpha: f32) {
        let font = self.font.as_scaled(PxScale::from(size));
        let baseline = y + font.ascent();
        let mut caret = x;
        for character in text.chars() {
            let id = font.glyph_id(character);
            let glyph = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(caret, baseline));
            caret += font.h_advance(id);
            let Some(outline) = self.font.outline_glyph(glyph) else {
                continue;
            };
            blend(pixmap, &outline, color, alpha);
        }
    }
}

/// Where a finger is at `t`: on the line from each place to the next, as
/// an agent's swipe gives only where it starts and ends.
fn finger_at(finger: &Finger, t: u64) -> Place {
    let next = finger.path.partition_point(|place| place.t <= t);
    let last = finger.path[next.saturating_sub(1)];
    match finger.path.get(next) {
        Some(coming) if next > 0 => {
            let share = (t - last.t) as f32 / (coming.t - last.t).max(1) as f32;
            Place {
                t,
                x: last.x + (coming.x - last.x) * share,
                y: last.y + (coming.y - last.y) * share,
                scale: coming.scale,
            }
        }
        _ => last,
    }
}

/// How wide the edge that rounds the arrowhead's corners is, in points.
const EDGE: f32 = 1.2;

/// The color a press of `button` shows in.
fn button_color(button: InputButton) -> [u8; 3] {
    match button {
        InputButton::Left => BLUE,
        InputButton::Right => ORANGE,
        InputButton::Middle => GREY,
    }
}

/// The pointer at `place`, `size` of its full size: an arrowhead with no
/// tail, its tip on the point, `fill` in a white outline over a soft shadow,
/// as screen recorders draw it to read over any page.
fn draw_arrow(pixmap: &mut PixmapMut<'_>, place: &Place, size: f32, fill: [u8; 3]) {
    // The arrowhead's outline in points, its tip at the origin.
    const HEAD: [(f32, f32); 4] = [(0.0, 0.0), (0.0, 19.0), (5.5, 14.0), (13.5, 13.5)];
    let s = place.scale * size * 1.5;
    let outline = |dx: f32, dy: f32| {
        let mut path = PathBuilder::new();
        for (index, (x, y)) in HEAD.iter().enumerate() {
            let (px, py) = (place.x + dx + x * s, place.y + dy + y * s);
            if index == 0 {
                path.move_to(px, py);
            } else {
                path.line_to(px, py);
            }
        }
        path.close();
        path.finish()
    };
    // The white edge shows 1.75 points beyond the black, which a stroke
    // reaches half its width out from.
    let white = Stroke {
        width: (3.5 + EDGE) * s,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    // A soft shadow below the white edge, so the pointer stands off a white
    // page as well as a dark one: strokes ever wider and fainter, as a blur
    // of SHADOW points would draw it.
    const SHADOW: f32 = 1.2;
    const LAYERS: usize = 5;
    if let Some(shadow) = outline(0.0, 1.0 * s) {
        pixmap.fill_path(&shadow, &paint([0, 0, 0], 0.07), FillRule::Winding, Transform::identity(), None);
        for layer in 1..=LAYERS {
            let spread = Stroke {
                width: white.width + 2.0 * SHADOW * s * layer as f32 / LAYERS as f32,
                ..white.clone()
            };
            pixmap.stroke_path(&shadow, &paint([0, 0, 0], 0.07), &spread, Transform::identity(), None);
        }
    }
    let Some(head) = outline(0.0, 0.0) else {
        return;
    };
    pixmap.stroke_path(&head, &paint([255, 255, 255], 1.0), &white, Transform::identity(), None);
    pixmap.fill_path(&head, &paint(fill, 1.0), FillRule::Winding, Transform::identity(), None);
    let edge = Stroke {
        width: EDGE * s,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pixmap.stroke_path(&head, &paint(fill, 1.0), &edge, Transform::identity(), None);
}

pub(crate) fn paint(color: [u8; 3], alpha: f32) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(color[0], color[1], color[2], (alpha.clamp(0.0, 1.0) * 255.0) as u8));
    paint.anti_alias = true;
    paint
}

pub(crate) fn rounded(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Option<tiny_skia::Path> {
    let radius = radius.min(width / 2.0).min(height / 2.0);
    let mut path = PathBuilder::new();
    path.move_to(x + radius, y);
    path.line_to(x + width - radius, y);
    path.quad_to(x + width, y, x + width, y + radius);
    path.line_to(x + width, y + height - radius);
    path.quad_to(x + width, y + height, x + width - radius, y + height);
    path.line_to(x + radius, y + height);
    path.quad_to(x, y + height, x, y + height - radius);
    path.line_to(x, y + radius);
    path.quad_to(x, y, x + radius, y);
    path.close();
    path.finish()
}

/// A key as the capsule names it: by the symbol macOS uses on a Mac, where
/// the export runs on the Host that recorded, and by its name elsewhere.
fn key_name(key: &str) -> String {
    let mac = cfg!(target_os = "macos");
    let named = match key {
        "Meta" if mac => "⌘",
        "Control" if mac => "⌃",
        "Alt" if mac => "⌥",
        "Shift" if mac => "⇧",
        "Enter" | "enter" if mac => "↩",
        "Backspace" if mac => "⌫",
        "Delete" | "delete" if mac => "⌦",
        "Tab" | "tab" if mac => "⇥",
        "Control" => "Ctrl",
        "Meta" => "Win",
        "Escape" | "escape" => "esc",
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        " " => "Space",
        "enter" => "Enter",
        "delete" => "Delete",
        "tab" => "Tab",
        other => return other.to_owned(),
    };
    named.to_owned()
}

/// A device's button as the capsule names it.
fn button_name(name: &str) -> String {
    match name {
        "home" => "Home".into(),
        "back" => "Back".into(),
        "lock" => "Lock".into(),
        "volume-up" => "Volume +".into(),
        "volume-down" => "Volume -".into(),
        other => other.to_owned(),
    }
}

/// Blends a glyph's coverage over the picture.
pub(crate) fn blend(pixmap: &mut PixmapMut<'_>, outline: &ab_glyph::OutlinedGlyph, color: [u8; 3], alpha: f32) {
    let bounds = outline.px_bounds();
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    let pixels = pixmap.data_mut();
    outline.draw(|gx, gy, coverage| {
        let (px, py) = (bounds.min.x as i32 + gx as i32, bounds.min.y as i32 + gy as i32);
        if px < 0 || py < 0 || px >= width || py >= height {
            return;
        }
        let a = (coverage * alpha).clamp(0.0, 1.0);
        let at = (py as usize * width as usize + px as usize) * 4;
        for (channel, value) in color.iter().enumerate() {
            let under = f32::from(pixels[at + channel]);
            pixels[at + channel] = (under + (f32::from(*value) - under) * a).round() as u8;
        }
    });
}
