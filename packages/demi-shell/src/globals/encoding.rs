//! UTF-8 and Latin-1 transcoding behind `TextEncoder`, `TextDecoder`,
//! `atob` and `btoa`.

use rquickjs::{Array, Ctx, Exception, Result, TypedArray, Value};

pub fn utf8_encode<'js>(ctx: Ctx<'js>, text: String) -> Result<TypedArray<'js, u8>> {
    TypedArray::new(ctx, text.into_bytes())
}

/// Decodes `bytes`; with `stream` an incomplete trailing sequence is
/// returned as the second element instead of being an error. With `fatal`
/// invalid input throws a `TypeError`; otherwise it becomes U+FFFD.
pub fn utf8_decode<'js>(
    ctx: Ctx<'js>,
    bytes: TypedArray<'js, u8>,
    fatal: bool,
    stream: bool,
) -> Result<Array<'js>> {
    let input = bytes.as_bytes().unwrap_or(&[]);
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    let mut remainder: Option<Vec<u8>> = None;
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                break;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                out.push_str(std::str::from_utf8(&rest[..valid]).expect("validated prefix"));
                match e.error_len() {
                    None if stream => {
                        remainder = Some(rest[valid..].to_vec());
                        break;
                    }
                    None => {
                        if fatal {
                            return Err(Exception::throw_type(&ctx, "The encoded data was not valid for encoding utf-8"));
                        }
                        out.push('\u{FFFD}');
                        break;
                    }
                    Some(bad) => {
                        if fatal {
                            return Err(Exception::throw_type(&ctx, "The encoded data was not valid for encoding utf-8"));
                        }
                        out.push('\u{FFFD}');
                        rest = &rest[valid + bad..];
                    }
                }
            }
        }
    }
    let result = Array::new(ctx.clone())?;
    result.set(0, out)?;
    match remainder {
        Some(r) => result.set(1, TypedArray::new(ctx, r)?)?,
        None => result.set(1, Value::new_null(ctx))?,
    }
    Ok(result)
}

/// Latin-1 (`btoa` input): every char must be below 256.
pub fn latin1_encode<'js>(ctx: Ctx<'js>, text: String) -> Result<TypedArray<'js, u8>> {
    let mut out = Vec::with_capacity(text.len());
    for c in text.chars() {
        let code = c as u32;
        if code > 0xff {
            return Err(Exception::throw_message(&ctx, "Invalid character"));
        }
        out.push(code as u8);
    }
    TypedArray::new(ctx, out)
}

pub fn latin1_decode(bytes: TypedArray<'_, u8>) -> String {
    bytes.as_bytes().unwrap_or(&[]).iter().map(|b| *b as char).collect()
}
