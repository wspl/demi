//! Android's compiled XML, as an APK holds a layout or a drawable: chunks
//! of a string pool, the attributes' resource ids, and each element's start
//! and end with its typed attribute values (`ResourceTypes.h` in AOSP's
//! `libandroidfw`, whose layout this follows).

use std::sync::Arc;

use thiserror::Error;

#[derive(Debug, Error)]
#[error("compiled XML does not read: {0}")]
pub struct Unreadable(String);

const STRING_POOL: u16 = 0x0001;
const XML: u16 = 0x0003;
const START_ELEMENT: u16 = 0x0102;
const END_ELEMENT: u16 = 0x0103;
const RESOURCE_MAP: u16 = 0x0180;
/// The string pool's strings are UTF-8, not UTF-16.
const UTF8: u32 = 1 << 8;
/// A string index that names no string.
const NONE: u32 = u32::MAX;
/// The size of an attribute's fields (`ResXMLTree_attribute`).
const ATTRIBUTE: usize = 20;
/// The most strings, elements and attributes a document may hold
/// together: a corner's drawable holds a few dozen.
const ITEMS: usize = 65_536;

/// An attribute's value, by its `Res_value` type.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(Arc<str>),
    Float(f32),
    /// A size with its unit: `px` 0, `dp` 1, `sp` 2, `pt` 3, `in` 4, `mm` 5.
    Dimension(f32, u8),
    Integer(i32),
    /// A colour, ARGB.
    Color(u32),
    /// Another resource, by id.
    Reference(u32),
    /// A type this reader has no use for.
    Other(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: Arc<str>,
    /// The attribute's resource id, such as `android:width`'s 0x01010159.
    pub id: Option<u32>,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Start { name: Arc<str>, attributes: Vec<Attribute> },
    End { name: Arc<str> },
}

/// Reads little-endian fields of `bytes`.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn u16(&self, at: usize) -> Result<u16, Unreadable> {
        let field = self.bytes.get(at..at + 2).ok_or_else(|| Unreadable(format!("ends before byte {at}")))?;
        Ok(u16::from_le_bytes([field[0], field[1]]))
    }

    fn u32(&self, at: usize) -> Result<u32, Unreadable> {
        let field = self.bytes.get(at..at + 4).ok_or_else(|| Unreadable(format!("ends before byte {at}")))?;
        Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
    }

    fn u8(&self, at: usize) -> Result<u8, Unreadable> {
        self.bytes.get(at).copied().ok_or_else(|| Unreadable(format!("ends before byte {at}")))
    }
}

/// The elements of the compiled XML `bytes`, in document order. Every
/// size and place the bytes state is checked against the bytes, and no
/// more than [`ITEMS`] strings, elements and attributes are read, so what
/// is read is never larger than a small multiple of the bytes.
pub fn events(bytes: &[u8]) -> Result<Vec<Event>, Unreadable> {
    let reader = Reader { bytes };
    if reader.u16(0)? != XML {
        return Err(Unreadable("not an XML chunk".into()));
    }
    let end = reader.u32(4)? as usize;
    if end != bytes.len() {
        return Err(Unreadable(format!("says it is {end} bytes, but is {}", bytes.len())));
    }
    let mut at = reader.u16(2)? as usize;
    if at < 8 || at > end {
        return Err(Unreadable(format!("its header of {at} bytes does not fit")));
    }
    let mut strings: Vec<Arc<str>> = Vec::new();
    let mut ids = Vec::new();
    let mut events = Vec::new();
    let mut items = 0usize;
    let mut count = |more: usize| {
        items += more;
        if items > ITEMS { Err(Unreadable(format!("holds more than {ITEMS} items"))) } else { Ok(()) }
    };
    while at < end {
        let kind = reader.u16(at)?;
        let header = reader.u16(at + 2)? as usize;
        let size = reader.u32(at + 4)? as usize;
        if header < 8 || header > size || size > end - at {
            return Err(Unreadable(format!("a chunk at byte {at} does not fit")));
        }
        let chunk = Reader { bytes: &bytes[at..at + size] };
        match kind {
            STRING_POOL => {
                strings = string_pool(&chunk, header)?;
                count(strings.len())?;
            }
            RESOURCE_MAP => {
                ids = (header..size - size % 4).step_by(4).map(|offset| chunk.u32(offset)).collect::<Result<_, _>>()?;
                count(ids.len())?;
            }
            START_ELEMENT => {
                // After the node header: the element's namespace and name,
                // where its attributes start, their size and their count.
                let name = string(&strings, chunk.u32(header + 4)?)?;
                let start = chunk.u16(header + 8)? as usize;
                let stride = chunk.u16(header + 10)? as usize;
                let attributes = chunk.u16(header + 12)? as usize;
                let first = header + start;
                if stride < ATTRIBUTE || first + attributes * stride > size {
                    return Err(Unreadable(format!("the attributes at byte {at} do not fit their element")));
                }
                count(1 + attributes)?;
                let attributes = (0..attributes)
                    .map(|index| {
                        let attribute = first + index * stride;
                        let name_index = chunk.u32(attribute + 4)?;
                        let raw = chunk.u32(attribute + 8)?;
                        let kind = chunk.u8(attribute + 15)?;
                        let data = chunk.u32(attribute + 16)?;
                        Ok(Attribute {
                            name: string(&strings, name_index)?,
                            id: ids.get(name_index as usize).copied(),
                            value: value(&strings, raw, kind, data)?,
                        })
                    })
                    .collect::<Result<_, Unreadable>>()?;
                events.push(Event::Start { name, attributes });
            }
            END_ELEMENT => {
                count(1)?;
                events.push(Event::End {
                    name: string(&strings, chunk.u32(header + 4)?)?,
                });
            }
            // Namespaces and text play no part in what is read here.
            _ => {}
        }
        at += size;
    }
    Ok(events)
}

fn string(strings: &[Arc<str>], index: u32) -> Result<Arc<str>, Unreadable> {
    if index == NONE {
        return Ok(Arc::from(""));
    }
    strings
        .get(index as usize)
        .cloned()
        .ok_or_else(|| Unreadable(format!("string {index} is not in the pool")))
}

fn value(strings: &[Arc<str>], raw: u32, kind: u8, data: u32) -> Result<Value, Unreadable> {
    Ok(match kind {
        0x01 => Value::Reference(data),
        0x03 => Value::Text(string(strings, if raw != NONE { raw } else { data })?),
        0x04 => Value::Float(f32::from_bits(data)),
        0x05 => Value::Dimension(complex(data), (data & 0xf) as u8),
        0x10 | 0x11 | 0x12 => Value::Integer(data as i32),
        0x1c..=0x1f => Value::Color(data),
        other => Value::Other(other),
    })
}

/// A complex value's number: a 24-bit mantissa with one of four radixes
/// (`Res_value::COMPLEX_*`).
fn complex(data: u32) -> f32 {
    const RADIX: [f32; 4] = [1.0 / 256.0, 1.0 / 32768.0, 1.0 / 8_388_608.0, 1.0 / 2_147_483_648.0];
    ((data & 0xffff_ff00) as i32) as f32 * RADIX[((data >> 4) & 3) as usize]
}

/// The strings of the pool `chunk`, whose header is `header` bytes; each
/// lies within the chunk, and together they are no longer than twice it,
/// however many of them name the same bytes.
fn string_pool(chunk: &Reader<'_>, header: usize) -> Result<Vec<Arc<str>>, Unreadable> {
    let count = chunk.u32(8)? as usize;
    let utf8 = chunk.u32(16)? & UTF8 != 0;
    let start = chunk.u32(20)? as usize;
    let size = chunk.bytes.len();
    // Each string has an offset of four bytes.
    if count > (size - header) / 4 || start > size {
        return Err(Unreadable(format!("a pool of {count} strings does not fit its {size} bytes")));
    }
    let mut left = size * 2;
    (0..count)
        .map(|index| {
            let at = start
                .checked_add(chunk.u32(header + index * 4)? as usize)
                .ok_or_else(|| Unreadable("a string lies past its pool".into()))?;
            let text = if utf8 { utf8_string(chunk, at, &mut left) } else { utf16_string(chunk, at, &mut left) }?;
            Ok(Arc::from(text))
        })
        .collect()
}

/// A UTF-8 string: its length in UTF-16 units, then in bytes, each one or
/// two bytes, then the bytes.
fn utf8_string(chunk: &Reader<'_>, at: usize, left: &mut usize) -> Result<String, Unreadable> {
    let length = |at: usize| -> Result<(usize, usize), Unreadable> {
        let first = chunk.u8(at)? as usize;
        Ok(if first & 0x80 != 0 { (((first & 0x7f) << 8) | chunk.u8(at + 1)? as usize, 2) } else { (first, 1) })
    };
    let (_, skip) = length(at)?;
    let (bytes, width) = length(at + skip)?;
    let from = at + skip + width;
    let text = chunk.bytes.get(from..from + bytes).ok_or_else(|| Unreadable("a string runs past its pool".into()))?;
    spend(left, text.len())?;
    String::from_utf8(text.to_vec()).map_err(|error| Unreadable(error.to_string()))
}

/// A UTF-16 string: its length in units, one or two units, then the units.
fn utf16_string(chunk: &Reader<'_>, at: usize, left: &mut usize) -> Result<String, Unreadable> {
    let first = chunk.u16(at)? as usize;
    let (units, from) = if first & 0x8000 != 0 {
        ((((first & 0x7fff) << 16) | chunk.u16(at + 2)? as usize), at + 4)
    } else {
        (first, at + 2)
    };
    let text = chunk
        .bytes
        .get(from..from.saturating_add(units.saturating_mul(2)))
        .ok_or_else(|| Unreadable("a string runs past its pool".into()))?;
    // UTF-8 takes at most three bytes for a unit's two.
    spend(left, text.len() / 2 * 3)?;
    let units: Vec<u16> = text.chunks_exact(2).map(|unit| u16::from_le_bytes([unit[0], unit[1]])).collect();
    String::from_utf16(&units).map_err(|error| Unreadable(error.to_string()))
}

/// Takes `bytes` of what is `left` for the pool's strings.
fn spend(left: &mut usize, bytes: usize) -> Result<(), Unreadable> {
    *left = left
        .checked_sub(bytes)
        .ok_or_else(|| Unreadable("the pool's strings are longer than the pool".into()))?;
    Ok(())
}
