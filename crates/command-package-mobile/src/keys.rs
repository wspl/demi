//! A hardware keyboard's keys as USB HID usages (HID Usage Tables § 10,
//! Keyboard/Keypad Page), which a simulator takes: the viewer's keys by
//! their `KeyboardEvent.code`, and the characters `type` types on a US
//! layout, with Shift where the character needs it.

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
