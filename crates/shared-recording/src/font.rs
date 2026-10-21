//! The little text a labelled image carries, drawn from a 5 × 7 bitmap font
//! of the characters a frame number and a time use, so an exported image
//! needs no font file.

const WIDTH: usize = 5;
const HEIGHT: usize = 7;

/// A character's rows, top first, each as the five bits of its pixels, the
/// leftmost the highest.
fn glyph(character: char) -> [u8; HEIGHT] {
    match character {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        'f' => [0b00110, 0b01001, 0b01000, 0b11100, 0b01000, 0b01000, 0b01000],
        's' => [0b00000, 0b00000, 0b01111, 0b10000, 0b01110, 0b00001, 0b11110],
        'm' => [0b00000, 0b00000, 0b11010, 0b10101, 0b10101, 0b10101, 0b10101],
        '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        _ => [0; HEIGHT],
    }
}

/// How wide `text` draws at `scale` pixels a font pixel, and how tall.
pub fn size(text: &str, scale: usize) -> (usize, usize) {
    let characters = text.chars().count();
    ((characters * (WIDTH + 1)).saturating_sub(1) * scale, HEIGHT * scale)
}

/// Draws `text` into `canvas`, `canvas_width` pixels of RGBA a row, its
/// top-left corner at `x`, `y`.
pub fn draw(canvas: &mut [u8], canvas_width: usize, x: usize, y: usize, text: &str, scale: usize, colour: [u8; 4]) {
    for (index, character) in text.chars().enumerate() {
        for (row, bits) in glyph(character).into_iter().enumerate() {
            for column in 0..WIDTH {
                if bits & (1 << (WIDTH - 1 - column)) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = x + (index * (WIDTH + 1) + column) * scale + dx;
                        let py = y + row * scale + dy;
                        let at = (py * canvas_width + px) * 4;
                        canvas[at..at + 4].copy_from_slice(&colour);
                    }
                }
            }
        }
    }
}
