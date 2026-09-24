//! Pairing codes and device tokens (`backend.md` § Authentication and
//! ownership). A pairing code is 128 random bits a waiting runner prints for
//! its user, so guessing one is infeasible by its entropy alone; it is
//! single use and short-lived by the pending claims' rules. A device token is
//! 256 random bits in hexadecimal, which only the runner receives; storage
//! keeps its SHA-256.

use std::sync::LazyLock;

use data_encoding::{BitOrder, Encoding, Specification};
use demi_runner_protocol::values::DeviceToken;

/// Crockford's base32: no `I`, `L`, `O` or `U`, so a code reads aloud and
/// types without confusion. Decoding takes either case, reads `O` as `0` and
/// `I` and `L` as `1`, and ignores the dashes and spaces a person types.
static CROCKFORD: LazyLock<Encoding> = LazyLock::new(|| {
    let mut specification = Specification::new();
    specification.symbols.push_str("0123456789ABCDEFGHJKMNPQRSTVWXYZ");
    specification.bit_order = BitOrder::MostSignificantFirst;
    specification.check_trailing_bits = true;
    specification.ignore.push_str(" \t\r\n-");
    specification.translate.from.push_str("abcdefghjkmnpqrstvwxyzoOiIlL");
    specification.translate.to.push_str("ABCDEFGHJKMNPQRSTVWXYZ001111");
    specification.encoding().expect("the Crockford alphabet is a valid base32")
});

/// How many characters of a printed code go between two dashes.
const GROUP: usize = 4;

/// A pairing code: its bits, however its user spells them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ClaimCode([u8; 16]);

impl ClaimCode {
    pub(crate) fn generate() -> Self {
        Self(rand::random())
    }

    /// The code a user typed, when it is one: 26 characters of the alphabet
    /// in any case, with any dashes and spaces.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let bytes = CROCKFORD.decode(text.as_bytes()).ok()?;
        bytes.try_into().ok().map(Self)
    }

    /// The code as the runner prints it, in groups of four.
    pub(crate) fn printed(&self) -> String {
        let text = CROCKFORD.encode(&self.0);
        let groups: Vec<&str> = text
            .as_bytes()
            .chunks(GROUP)
            .map(|group| std::str::from_utf8(group).expect("base32 is ASCII"))
            .collect();
        groups.join("-")
    }
}

/// A new device's credential.
pub(crate) fn new_device_token() -> DeviceToken {
    let bits: [u8; 32] = rand::random();
    DeviceToken::try_from(hex::encode(bits)).expect("64 hexadecimal digits are a device token")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_prints_in_groups_and_parses_however_it_is_typed() {
        let code = ClaimCode::generate();
        let printed = code.printed();
        assert_eq!(printed.len(), 26 + 6);
        assert!(printed.split('-').all(|group| (1..=GROUP).contains(&group.len())));
        assert!(printed.chars().all(|char| char == '-' || "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(char)));
        assert_eq!(ClaimCode::parse(&printed), Some(code));
        let messy = format!(" {} ", printed.to_lowercase().replace('-', " "));
        assert_eq!(ClaimCode::parse(&messy), Some(code));
        let confusable = printed.replace('0', "o").replace('1', "l");
        assert_eq!(ClaimCode::parse(&confusable), Some(code));
        for refused in ["", "AAAA-BBBB", "NOPE-NOPE", &printed[..printed.len() - 1], "U".repeat(26).as_str()] {
            assert_eq!(ClaimCode::parse(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_device_token_is_256_bits_in_hexadecimal() {
        let token = new_device_token();
        assert_eq!(token.expose().len(), 64);
        assert!(token.expose().chars().all(|char| char.is_ascii_hexdigit()));
        assert_ne!(token.expose(), new_device_token().expose());
    }
}
