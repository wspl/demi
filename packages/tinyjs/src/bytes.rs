//! `tinyjs:bytes`: the per-byte codecs — MessagePack, base64, SHA-256,
//! random bytes — over `rmpv`, `base64` and `sha2`. JS on tinyjs never
//! iterates over a chunk.

use base64::Engine;
use rquickjs::function::Func;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{Array, Ctx, Exception, Function, Object, Result, TypedArray, Value};
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

pub fn base64_encode(data: TypedArray<'_, u8>) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes_of(&data))
}

/// Accepts the standard and URL-safe alphabets, with or without padding.
pub fn base64_decode<'js>(ctx: Ctx<'js>, text: String) -> Result<TypedArray<'js, u8>> {
    let normalized: String = text
        .chars()
        .filter(|c| *c != '=')
        .map(|c| match c { '-' => '+', '_' => '/', c => c })
        .collect();
    let decoded = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(normalized)
        .map_err(|_| Exception::throw_message(&ctx, "The string to be decoded is not correctly encoded."))?;
    TypedArray::new(ctx, decoded)
}

fn sha256<'js>(ctx: Ctx<'js>, data: TypedArray<'js, u8>) -> Result<TypedArray<'js, u8>> {
    TypedArray::new(ctx, Sha256::digest(bytes_of(&data)).to_vec())
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
    let date: Function = ctx.globals().get("Date")?;
    let tree = to_msgpack(&ctx, &value, &date, 0)?;
    let mut out = Vec::with_capacity(256);
    rmpv::encode::write_value(&mut out, &tree).map_err(|e| Exception::throw_type(&ctx, &format!("msgpackEncode: {e}")))?;
    TypedArray::new(ctx, out)
}

fn to_msgpack<'js>(ctx: &Ctx<'js>, value: &Value<'js>, date: &Function<'js>, depth: u32) -> Result<rmpv::Value> {
    use rmpv::Value as M;
    let fail = |m: &str| Exception::throw_type(ctx, m);
    if depth > 100 {
        return Err(fail("msgpackEncode: object nesting too deep"));
    }
    if value.is_null() || value.is_undefined() {
        return Ok(M::Nil);
    }
    if let Some(b) = value.as_bool() {
        return Ok(M::Boolean(b));
    }
    if let Some(i) = value.as_int() {
        return Ok(M::from(i));
    }
    if let Some(f) = value.as_float() {
        return Ok(if f.fract() == 0.0 && f.abs() < 9007199254740992.0 {
            M::from(f as i64)
        } else {
            M::F64(f)
        });
    }
    if let Some(s) = value.as_string() {
        return Ok(M::from(s.to_string()?));
    }
    let Some(obj) = value.as_object() else {
        return Err(fail("msgpackEncode: unsupported value"));
    };
    if let Some(bytes) = obj.as_typed_array::<u8>() {
        return Ok(M::Binary(bytes_of(bytes).to_vec()));
    }
    if let Some(arr) = value.as_array() {
        let mut items = Vec::with_capacity(arr.len());
        for item in arr.iter::<Value>() {
            items.push(to_msgpack(ctx, &item?, date, depth + 1)?);
        }
        return Ok(M::Array(items));
    }
    if obj.is_instance_of(date) {
        let ms: f64 = obj.get::<_, Function>("getTime")?.call((rquickjs::function::This(obj.clone()),))?;
        return Ok(M::Ext(-1, timestamp_bytes(ms)));
    }
    if obj.is_function() {
        return Err(fail("msgpackEncode: functions cannot be encoded"));
    }
    let mut entries = Vec::new();
    for r in obj.props::<String, Value>() {
        let (k, v) = r?;
        entries.push((M::from(k), to_msgpack(ctx, &v, date, depth + 1)?));
    }
    Ok(M::Map(entries))
}

/// The msgpack timestamp extension in its shortest form.
fn timestamp_bytes(ms: f64) -> Vec<u8> {
    let secs = (ms / 1000.0).floor();
    let nanos = ((ms - secs * 1000.0) * 1e6).round() as u32;
    let secs = secs as i64;
    if secs >= 0 && secs < (1i64 << 34) {
        if nanos == 0 && secs < (1i64 << 32) {
            (secs as u32).to_be_bytes().to_vec()
        } else {
            (((nanos as u64) << 34) | secs as u64).to_be_bytes().to_vec()
        }
    } else {
        let mut out = nanos.to_be_bytes().to_vec();
        out.extend_from_slice(&secs.to_be_bytes());
        out
    }
}

fn timestamp_ms(data: &[u8]) -> Option<f64> {
    Some(match data.len() {
        4 => u32::from_be_bytes(data.try_into().ok()?) as f64 * 1000.0,
        8 => {
            let v = u64::from_be_bytes(data.try_into().ok()?);
            (v & 0x3_ffff_ffff) as f64 * 1000.0 + (v >> 34) as f64 / 1e6
        }
        12 => {
            let nanos = u32::from_be_bytes(data[..4].try_into().ok()?);
            let secs = i64::from_be_bytes(data[4..].try_into().ok()?);
            secs as f64 * 1000.0 + nanos as f64 / 1e6
        }
        _ => return None,
    })
}

fn msgpack_decode<'js>(ctx: Ctx<'js>, data: TypedArray<'js, u8>) -> Result<Value<'js>> {
    let mut cursor = bytes_of(&data);
    let tree = rmpv::decode::read_value_with_max_depth(&mut cursor, 100)
        .map_err(|e| Exception::throw_type(&ctx, &format!("msgpackDecode: {e}")))?;
    if !cursor.is_empty() {
        return Err(Exception::throw_type(&ctx, "msgpackDecode: trailing bytes"));
    }
    let date = rquickjs::function::Constructor::from_value(ctx.globals().get::<_, Value>("Date")?)?;
    from_msgpack(&ctx, tree, &date)
}

fn from_msgpack<'js>(ctx: &Ctx<'js>, value: rmpv::Value, date: &rquickjs::function::Constructor<'js>) -> Result<Value<'js>> {
    use rmpv::Value as M;
    Ok(match value {
        M::Nil => Value::new_null(ctx.clone()),
        M::Boolean(b) => Value::new_bool(ctx.clone(), b),
        M::Integer(i) => Value::new_number(ctx.clone(), i.as_f64().unwrap_or(f64::NAN)),
        M::F32(f) => Value::new_number(ctx.clone(), f as f64),
        M::F64(f) => Value::new_number(ctx.clone(), f),
        M::String(s) => rquickjs::String::from_str(ctx.clone(), &String::from_utf8_lossy(s.as_bytes()))?.into_value(),
        M::Binary(b) => TypedArray::new(ctx.clone(), b)?.into_value(),
        M::Array(items) => {
            let arr = Array::new(ctx.clone())?;
            for (i, item) in items.into_iter().enumerate() {
                arr.set(i, from_msgpack(ctx, item, date)?)?;
            }
            arr.into_value()
        }
        M::Map(entries) => {
            let obj = Object::new(ctx.clone())?;
            for (k, v) in entries {
                let key = match k {
                    M::String(s) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
                    M::Integer(i) => i.to_string(),
                    _ => return Err(Exception::throw_type(ctx, "msgpackDecode: map keys must be strings")),
                };
                obj.set(key, from_msgpack(ctx, v, date)?)?;
            }
            obj.into_value()
        }
        M::Ext(-1, data) => {
            let ms = timestamp_ms(&data).ok_or_else(|| Exception::throw_type(ctx, "msgpackDecode: invalid timestamp"))?;
            date.construct::<_, Value>((ms,))?
        }
        M::Ext(ty, _) => return Err(Exception::throw_type(ctx, &format!("msgpackDecode: unknown extension type {ty}"))),
    })
}
