//! The little of the H.264 bitstream that the programs reading or rewriting a
//! parameter set need: Annex B framing, emulation prevention, and the bits of
//! a payload, which no crate the workspace has offers for writing an SPS.

use std::ops::Range;

/// The NAL unit type of a sequence parameter set.
pub const SPS: u8 = 7;

/// The type of a NAL unit, from its header byte.
pub fn kind(unit: &[u8]) -> Option<u8> {
    unit.first().map(|header| header & 0x1f)
}

/// The NAL units of an Annex B stream, without their start codes.
pub fn units(stream: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut at = 0;
    while at + 3 <= stream.len() {
        if stream[at..at + 3] == [0, 0, 1] {
            starts.push((at, at + 3));
            at += 3;
        } else {
            at += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (index, &(_, begin)) in starts.iter().enumerate() {
        let mut end = starts.get(index + 1).map_or(stream.len(), |&(next, _)| next);
        // A four-byte start code's leading zero, and trailing zeros, are no unit's.
        while end > begin && stream[end - 1] == 0 {
            end -= 1;
        }
        units.push(&stream[begin..end]);
    }
    units
}

/// `units` as an Annex B stream, each after a four-byte start code.
pub fn annex_b(units: &[impl AsRef<[u8]>]) -> Vec<u8> {
    let mut stream = Vec::with_capacity(units.iter().map(|unit| unit.as_ref().len() + 4).sum());
    for unit in units {
        stream.extend_from_slice(&[0, 0, 0, 1]);
        stream.extend_from_slice(unit.as_ref());
    }
    stream
}

/// A NAL unit's payload without its emulation prevention bytes.
pub fn unescape(payload: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(payload.len());
    let mut zeros = 0;
    for &byte in payload {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        rbsp.push(byte);
    }
    rbsp
}

/// `rbsp` with the emulation prevention bytes a NAL unit needs, appended to
/// `unit`.
pub fn escape(rbsp: &[u8], unit: &mut Vec<u8>) {
    let mut zeros = 0;
    for &byte in rbsp {
        if zeros >= 2 && byte <= 3 {
            unit.push(3);
            zeros = 0;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        unit.push(byte);
    }
}

pub struct BitReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    /// How many bits were read.
    pub fn position(&self) -> usize {
        self.position
    }

    fn at(&self, position: usize) -> Option<u8> {
        let byte = self.bytes.get(position / 8)?;
        Some((byte >> (7 - position % 8)) & 1)
    }

    pub fn bits(&mut self, count: u32) -> Option<u32> {
        let mut value = 0;
        for _ in 0..count {
            value = value << 1 | u32::from(self.at(self.position)?);
            self.position += 1;
        }
        Some(value)
    }

    /// An unsigned Exp-Golomb value.
    pub fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bits(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1u32 << zeros) - 1 + self.bits(zeros)?)
    }

    /// A signed Exp-Golomb value, whose value no caller needs.
    pub fn se(&mut self) -> Option<()> {
        self.ue().map(drop)
    }
}

#[derive(Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    pub fn bit(&mut self, bit: u8) {
        if self.bits % 8 == 0 {
            self.bytes.push(0);
        }
        if bit == 1 {
            *self.bytes.last_mut().expect("a byte was pushed") |= 1 << (7 - self.bits % 8);
        }
        self.bits += 1;
    }

    /// An unsigned Exp-Golomb value.
    pub fn ue(&mut self, value: u32) {
        let coded = u64::from(value) + 1;
        let length = 64 - coded.leading_zeros();
        for _ in 1..length {
            self.bit(0);
        }
        for index in (0..length).rev() {
            self.bit(((coded >> index) & 1) as u8);
        }
    }

    /// The bits of `range` of `source`, which must lie in it.
    pub fn copy(&mut self, source: &[u8], range: Range<usize>) {
        let reader = BitReader::new(source);
        for position in range {
            self.bit(reader.at(position).expect("the range lies in the source"));
        }
    }

    /// The bytes, the last one padded with zeros.
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}
