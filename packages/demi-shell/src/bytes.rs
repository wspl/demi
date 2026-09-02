//! `demishell:bytes`: the per-byte codecs — MessagePack, base64, SHA-256,
//! random bytes. JS on the shell never iterates over a chunk.

use rquickjs::function::Func;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{Array, Ctx, Exception, FromJs, Function, Object, Result, TypedArray, Value};
use sha2::{Digest, Sha256};

pub struct BytesModule;

impl ModuleDef for BytesModule {
    fn declare(decl: &Declarations<'_>) -> Result<()> {
        for name in ["msgpackEncode", "msgpackDecode", "base64Encode", "base64Decode", "sha256", "randomBytes"] {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(_ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        exports.export("msgpackEncode", Func::from(msgpack_encode))?;
        exports.export("msgpackDecode", Func::from(msgpack_decode))?;
        exports.export("base64Encode", Func::from(base64_encode))?;
        exports.export("base64Decode", Func::from(base64_decode))?;
        exports.export("sha256", Func::from(sha256))?;
        exports.export("randomBytes", Func::from(random_bytes))?;
        Ok(())
    }
}

fn bytes_of<'a>(data: &'a TypedArray<'_, u8>) -> &'a [u8] {
    data.as_bytes().unwrap_or(&[])
}

// --- base64 -----------------------------------------------------------------

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: TypedArray<'_, u8>) -> String {
    let input = bytes_of(&data);
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

fn base64_value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some((c - b'A') as u32),
        b'a'..=b'z' => Some((c - b'a' + 26) as u32),
        b'0'..=b'9' => Some((c - b'0' + 52) as u32),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// Accepts standard and URL-safe alphabets, with or without padding.
pub fn base64_decode<'js>(ctx: Ctx<'js>, text: String) -> Result<TypedArray<'js, u8>> {
    let input = text.trim_end_matches('=').as_bytes();
    if input.len() % 4 == 1 {
        return Err(Exception::throw_message(&ctx, "The string to be decoded is not correctly encoded."));
    }
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &c in input {
        let v = base64_value(c)
            .ok_or_else(|| Exception::throw_message(&ctx, "The string to be decoded is not correctly encoded."))?;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    TypedArray::new(ctx, out)
}

// --- hashing and random --------------------------------------------------------

fn sha256<'js>(ctx: Ctx<'js>, data: TypedArray<'js, u8>) -> Result<TypedArray<'js, u8>> {
    let digest = Sha256::digest(bytes_of(&data));
    TypedArray::new(ctx, digest.to_vec())
}

fn random_bytes<'js>(ctx: Ctx<'js>, n: u32) -> Result<TypedArray<'js, u8>> {
    let mut out = vec![0u8; n as usize];
    getrandom::fill(&mut out).map_err(|e| crate::error::throw_code(&ctx, "EIO", &e.to_string(), "getrandom", None))?;
    TypedArray::new(ctx, out)
}

// --- MessagePack ----------------------------------------------------------------
//
// The mapping follows @msgpack/msgpack's defaults so both ends of the wire
// agree: integral numbers are ints, `Uint8Array` is bin, `Date` is the
// timestamp extension (-1), `undefined` is nil, object keys are strings.

fn msgpack_encode<'js>(ctx: Ctx<'js>, value: Value<'js>) -> Result<TypedArray<'js, u8>> {
    let mut out = Vec::with_capacity(256);
    let date: Function = ctx.globals().get("Date")?;
    encode_value(&ctx, &value, &date, &mut out, 0)?;
    TypedArray::new(ctx, out)
}

fn encode_value<'js>(ctx: &Ctx<'js>, value: &Value<'js>, date: &Function<'js>, out: &mut Vec<u8>, depth: u32) -> Result<()> {
    use rmp::encode as e;
    let fail = |m: &str| Exception::throw_type(ctx, m);
    if depth > 100 {
        return Err(fail("msgpackEncode: object nesting too deep"));
    }
    if value.is_null() || value.is_undefined() {
        e::write_nil(out).map_err(|_| fail("write"))?;
    } else if let Some(b) = value.as_bool() {
        e::write_bool(out, b).map_err(|_| fail("write"))?;
    } else if let Some(i) = value.as_int() {
        e::write_sint(out, i as i64).map_err(|_| fail("write"))?;
    } else if let Some(f) = value.as_float() {
        if f.fract() == 0.0 && f.abs() < 9007199254740992.0 {
            if f >= 0.0 {
                e::write_uint(out, f as u64).map_err(|_| fail("write"))?;
            } else {
                e::write_sint(out, f as i64).map_err(|_| fail("write"))?;
            }
        } else {
            e::write_f64(out, f).map_err(|_| fail("write"))?;
        }
    } else if let Some(s) = value.as_string() {
        let s = s.to_string()?;
        e::write_str(out, &s).map_err(|_| fail("write"))?;
    } else if let Some(bytes) = value.as_object().and_then(|o| o.as_typed_array::<u8>()) {
        e::write_bin(out, bytes_of(bytes)).map_err(|_| fail("write"))?;
    } else if let Some(arr) = value.as_array() {
        e::write_array_len(out, arr.len() as u32).map_err(|_| fail("write"))?;
        for item in arr.iter::<Value>() {
            encode_value(ctx, &item?, date, out, depth + 1)?;
        }
    } else if let Some(obj) = value.as_object() {
        if obj.is_instance_of(date) {
            let ms: f64 = obj.get::<_, Function>("getTime")?.call((rquickjs::function::This(obj.clone()),))?;
            write_timestamp(out, ms).map_err(|_| fail("write"))?;
        } else if obj.is_function() {
            return Err(fail("msgpackEncode: functions cannot be encoded"));
        } else {
            let mut entries: Vec<(String, Value<'js>)> = Vec::new();
            for r in obj.props::<String, Value>() {
                let (k, v) = r?;
                entries.push((k, v));
            }
            e::write_map_len(out, entries.len() as u32).map_err(|_| fail("write"))?;
            for (k, v) in entries {
                e::write_str(out, &k).map_err(|_| fail("write"))?;
                encode_value(ctx, &v, date, out, depth + 1)?;
            }
        }
    } else {
        return Err(fail("msgpackEncode: unsupported value"));
    }
    Ok(())
}

fn write_timestamp(out: &mut Vec<u8>, ms: f64) -> std::result::Result<(), rmp::encode::ValueWriteError> {
    use rmp::encode::write_ext_meta;
    let secs = (ms / 1000.0).floor();
    let nanos = ((ms - secs * 1000.0) * 1e6).round() as u32;
    let secs = secs as i64;
    if secs >= 0 && secs < (1i64 << 34) {
        if nanos == 0 && secs < (1i64 << 32) {
            write_ext_meta(out, 4, -1)?;
            out.extend_from_slice(&(secs as u32).to_be_bytes());
        } else {
            write_ext_meta(out, 8, -1)?;
            let data = ((nanos as u64) << 34) | secs as u64;
            out.extend_from_slice(&data.to_be_bytes());
        }
    } else {
        write_ext_meta(out, 12, -1)?;
        out.extend_from_slice(&nanos.to_be_bytes());
        out.extend_from_slice(&secs.to_be_bytes());
    }
    Ok(())
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        if end > self.data.len() {
            return None;
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> { self.take(1).map(|s| s[0]) }
    fn u16(&mut self) -> Option<u16> { self.take(2).map(|s| u16::from_be_bytes([s[0], s[1]])) }
    fn u32(&mut self) -> Option<u32> { self.take(4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])) }
    fn u64(&mut self) -> Option<u64> { self.take(8).map(|s| u64::from_be_bytes(s.try_into().unwrap())) }
}

fn msgpack_decode<'js>(ctx: Ctx<'js>, data: TypedArray<'js, u8>) -> Result<Value<'js>> {
    let mut cur = Cursor { data: bytes_of(&data), pos: 0 };
    let date: Function = ctx.globals().get("Date")?;
    let value = decode_value(&ctx, &mut cur, &date, 0)?;
    if cur.pos != cur.data.len() {
        return Err(Exception::throw_type(&ctx, "msgpackDecode: trailing bytes"));
    }
    Ok(value)
}

fn decode_value<'js>(ctx: &Ctx<'js>, cur: &mut Cursor<'_>, date: &Function<'js>, depth: u32) -> Result<Value<'js>> {
    use rmp::Marker;
    let truncated = || Exception::throw_type(ctx, "msgpackDecode: truncated input");
    if depth > 100 {
        return Err(Exception::throw_type(ctx, "msgpackDecode: nesting too deep"));
    }
    let marker = Marker::from_u8(cur.u8().ok_or_else(truncated)?);
    let number = |ctx: &Ctx<'js>, n: f64| -> Result<Value<'js>> { Ok(Value::new_number(ctx.clone(), n)) };
    let str_of = |cur: &mut Cursor<'_>, len: usize| -> Result<Value<'js>> {
        let s = cur.take(len).ok_or_else(truncated)?;
        let s = String::from_utf8_lossy(s);
        Ok(rquickjs::String::from_str(ctx.clone(), &s)?.into_value())
    };
    let bin_of = |cur: &mut Cursor<'_>, len: usize| -> Result<Value<'js>> {
        let b = cur.take(len).ok_or_else(truncated)?;
        Ok(TypedArray::new(ctx.clone(), b.to_vec())?.into_value())
    };
    let ext_of = |cur: &mut Cursor<'_>, len: usize| -> Result<Value<'js>> {
        let ty = cur.u8().ok_or_else(truncated)? as i8;
        let b = cur.take(len).ok_or_else(truncated)?;
        if ty != -1 {
            return Err(Exception::throw_type(ctx, &format!("msgpackDecode: unknown extension type {ty}")));
        }
        let ms = match len {
            4 => u32::from_be_bytes(b.try_into().unwrap()) as f64 * 1000.0,
            8 => {
                let v = u64::from_be_bytes(b.try_into().unwrap());
                (v & 0x3_ffff_ffff) as f64 * 1000.0 + (v >> 34) as f64 / 1e6
            }
            12 => {
                let nanos = u32::from_be_bytes(b[..4].try_into().unwrap());
                let secs = i64::from_be_bytes(b[4..].try_into().unwrap());
                secs as f64 * 1000.0 + nanos as f64 / 1e6
            }
            _ => return Err(Exception::throw_type(ctx, "msgpackDecode: invalid timestamp")),
        };
        let ctor = rquickjs::function::Constructor::from_value(date.clone().into_value())?;
        ctor.construct::<_, Value>((ms,))
    };
    let array_of = |cur: &mut Cursor<'_>, len: usize| -> Result<Value<'js>> {
        let arr = Array::new(ctx.clone())?;
        for i in 0..len {
            arr.set(i, decode_value(ctx, cur, date, depth + 1)?)?;
        }
        Ok(arr.into_value())
    };
    let map_of = |cur: &mut Cursor<'_>, len: usize| -> Result<Value<'js>> {
        let obj = Object::new(ctx.clone())?;
        for _ in 0..len {
            let key = decode_value(ctx, cur, date, depth + 1)?;
            let key = if let Some(s) = key.as_string() {
                s.to_string()?
            } else if let Some(n) = key.as_number() {
                n.to_string()
            } else {
                return Err(Exception::throw_type(ctx, "msgpackDecode: map keys must be strings"));
            };
            obj.set(key, decode_value(ctx, cur, date, depth + 1)?)?;
        }
        Ok(obj.into_value())
    };
    match marker {
        Marker::Null => Ok(Value::new_null(ctx.clone())),
        Marker::True => Ok(Value::new_bool(ctx.clone(), true)),
        Marker::False => Ok(Value::new_bool(ctx.clone(), false)),
        Marker::FixPos(n) => number(ctx, n as f64),
        Marker::FixNeg(n) => number(ctx, n as f64),
        Marker::U8 => number(ctx, cur.u8().ok_or_else(truncated)? as f64),
        Marker::U16 => number(ctx, cur.u16().ok_or_else(truncated)? as f64),
        Marker::U32 => number(ctx, cur.u32().ok_or_else(truncated)? as f64),
        Marker::U64 => number(ctx, cur.u64().ok_or_else(truncated)? as f64),
        Marker::I8 => number(ctx, cur.u8().ok_or_else(truncated)? as i8 as f64),
        Marker::I16 => number(ctx, cur.u16().ok_or_else(truncated)? as i16 as f64),
        Marker::I32 => number(ctx, cur.u32().ok_or_else(truncated)? as i32 as f64),
        Marker::I64 => number(ctx, cur.u64().ok_or_else(truncated)? as i64 as f64),
        Marker::F32 => number(ctx, f32::from_bits(cur.u32().ok_or_else(truncated)?) as f64),
        Marker::F64 => number(ctx, f64::from_bits(cur.u64().ok_or_else(truncated)?)),
        Marker::FixStr(n) => str_of(cur, n as usize),
        Marker::Str8 => { let n = cur.u8().ok_or_else(truncated)?; str_of(cur, n as usize) }
        Marker::Str16 => { let n = cur.u16().ok_or_else(truncated)?; str_of(cur, n as usize) }
        Marker::Str32 => { let n = cur.u32().ok_or_else(truncated)?; str_of(cur, n as usize) }
        Marker::Bin8 => { let n = cur.u8().ok_or_else(truncated)?; bin_of(cur, n as usize) }
        Marker::Bin16 => { let n = cur.u16().ok_or_else(truncated)?; bin_of(cur, n as usize) }
        Marker::Bin32 => { let n = cur.u32().ok_or_else(truncated)?; bin_of(cur, n as usize) }
        Marker::FixArray(n) => array_of(cur, n as usize),
        Marker::Array16 => { let n = cur.u16().ok_or_else(truncated)?; array_of(cur, n as usize) }
        Marker::Array32 => { let n = cur.u32().ok_or_else(truncated)?; array_of(cur, n as usize) }
        Marker::FixMap(n) => map_of(cur, n as usize),
        Marker::Map16 => { let n = cur.u16().ok_or_else(truncated)?; map_of(cur, n as usize) }
        Marker::Map32 => { let n = cur.u32().ok_or_else(truncated)?; map_of(cur, n as usize) }
        Marker::FixExt1 => ext_of(cur, 1),
        Marker::FixExt2 => ext_of(cur, 2),
        Marker::FixExt4 => ext_of(cur, 4),
        Marker::FixExt8 => ext_of(cur, 8),
        Marker::FixExt16 => ext_of(cur, 16),
        Marker::Ext8 => { let n = cur.u8().ok_or_else(truncated)?; ext_of(cur, n as usize) }
        Marker::Ext16 => { let n = cur.u16().ok_or_else(truncated)?; ext_of(cur, n as usize) }
        Marker::Ext32 => { let n = cur.u32().ok_or_else(truncated)?; ext_of(cur, n as usize) }
        Marker::Reserved => Err(Exception::throw_type(ctx, "msgpackDecode: reserved marker")),
    }
}

/// Reads a `Uint8Array` argument from a JS value (used by callers that
/// accept either bytes or another shape).
pub fn typed_array_from<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<TypedArray<'js, u8>> {
    TypedArray::from_js(ctx, value)
}
