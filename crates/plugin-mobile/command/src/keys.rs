//! A hardware keyboard's keys as USB HID usages (HID Usage Tables § 10,
//! Keyboard/Keypad Page), which a simulator takes.
//!
//! - The viewer's keys map by their `KeyboardEvent.code`.
//! - The characters `type` types map on a US layout, with Shift where the
//!   character needs it.
//! - Text a US keyboard cannot type goes in as the device's platform allows
//!   (`crates/plugin-mobile/command/src/operations.rs`), the agent's and the
//!   viewer's alike.

use std::sync::Arc;

use crate::driver::{Device, Result};

/// The usage of Left Shift.
pub const SHIFT: u32 = 0xe1;

/// The usage of the key `code` names, as `KeyboardEvent.code` names keys;
/// none for a key the simulator has no use for.
pub fn usage(code: &str) -> Option<u32> {
    if let Some(letter) = code.strip_prefix("Key")
        && let [letter] = letter.as_bytes()
        && letter.is_ascii_uppercase()
    {
        return Some(0x04 + u32::from(letter - b'A'));
    }
    if let Some(digit) = code.strip_prefix("Digit")
        && let [digit] = digit.as_bytes()
        && digit.is_ascii_digit()
    {
        return Some(match digit {
            b'0' => 0x27,
            digit => 0x1e + u32::from(digit - b'1'),
        });
    }
    Some(match code {
        "Enter" | "NumpadEnter" => 0x28,
        "Escape" => 0x29,
        "Backspace" => 0x2a,
        "Tab" => 0x2b,
        "Space" => 0x2c,
        "Minus" => 0x2d,
        "Equal" => 0x2e,
        "BracketLeft" => 0x2f,
        "BracketRight" => 0x30,
        "Backslash" => 0x31,
        "Semicolon" => 0x33,
        "Quote" => 0x34,
        "Backquote" => 0x35,
        "Comma" => 0x36,
        "Period" => 0x37,
        "Slash" => 0x38,
        "CapsLock" => 0x39,
        "Delete" => 0x4c,
        "ArrowRight" => 0x4f,
        "ArrowLeft" => 0x50,
        "ArrowDown" => 0x51,
        "ArrowUp" => 0x52,
        "Home" => 0x4a,
        "End" => 0x4d,
        "PageUp" => 0x4b,
        "PageDown" => 0x4e,
        "ShiftLeft" => SHIFT,
        "ShiftRight" => 0xe5,
        "ControlLeft" => 0xe0,
        "ControlRight" => 0xe4,
        "AltLeft" => 0xe2,
        "AltRight" => 0xe6,
        "MetaLeft" => 0xe3,
        "MetaRight" => 0xe7,
        _ => return None,
    })
}

/// The key that types `character` on a US layout, and whether it needs
/// Shift; none for a character no key types.
pub fn typed(character: char) -> Option<(u32, bool)> {
    const SHIFTED: &str = "!@#$%^&*()";
    if character.is_ascii_lowercase() {
        return Some((0x04 + u32::from(character as u8 - b'a'), false));
    }
    if character.is_ascii_uppercase() {
        return Some((0x04 + u32::from(character as u8 - b'A'), true));
    }
    if let Some(index) = SHIFTED.find(character) {
        let digit = b"1234567890"[index];
        return typed(digit as char).map(|(usage, _)| (usage, true));
    }
    Some(match character {
        '1'..='9' => (0x1e + u32::from(character as u8 - b'1'), false),
        '0' => (0x27, false),
        '\n' => (0x28, false),
        '\t' => (0x2b, false),
        ' ' => (0x2c, false),
        '-' => (0x2d, false),
        '_' => (0x2d, true),
        '=' => (0x2e, false),
        '+' => (0x2e, true),
        '[' => (0x2f, false),
        '{' => (0x2f, true),
        ']' => (0x30, false),
        '}' => (0x30, true),
        '\\' => (0x31, false),
        '|' => (0x31, true),
        ';' => (0x33, false),
        ':' => (0x33, true),
        '\'' => (0x34, false),
        '"' => (0x34, true),
        '`' => (0x35, false),
        '~' => (0x35, true),
        ',' => (0x36, false),
        '<' => (0x36, true),
        '.' => (0x37, false),
        '>' => (0x37, true),
        '/' => (0x38, false),
        '?' => (0x38, true),
        _ => return None,
    })
}

/// A key a US keyboard types a character with: its usage, and whether
/// Shift is held for it.
pub type Typed = (u32, bool);

/// A stretch of text as it goes in: typed, or inserted otherwise.
#[derive(Debug, PartialEq)]
pub enum Run {
    Keys(Vec<Typed>),
    Inserted(String),
}

/// `text` as it goes in: each stretch a US keyboard types as its keys, each
/// other stretch inserted as the platform allows.
pub fn runs(text: &str) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for character in text.chars() {
        match (typed(character), runs.last_mut()) {
            (Some(key), Some(Run::Keys(keys))) => keys.push(key),
            (Some(key), _) => runs.push(Run::Keys(vec![key])),
            (None, Some(Run::Inserted(inserted))) => inserted.push(character),
            (None, _) => runs.push(Run::Inserted(character.into())),
        }
    }
    runs
}

/// Enters `text` at the cursor of `device`: typed where a US keyboard has
/// the keys, the rest as the device's platform allows
/// (`crates/plugin-mobile/command/src/operations.rs`).
pub async fn enter(device: &Arc<dyn Device>, text: &str) -> Result<()> {
    for run in runs(text) {
        match run {
            Run::Keys(keys) => device.type_keys(&keys).await?,
            Run::Inserted(text) => device.insert(&text).await?,
        }
    }
    Ok(())
}
