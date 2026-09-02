//! The standard globals libraries look up by name.
//!
//! Per-byte work (encoders, random bytes, timers) is Rust and reachable from
//! the prelude through a temporary `__demishell_native` object; the prelude
//! (`prelude.js`) builds the JS-facing classes on top and removes the object.

mod encoding;
mod timers;

use rquickjs::function::Func;
use rquickjs::{Ctx, Object, Result, TypedArray};

use crate::bytes;
use crate::state::state;

const PRELUDE: &str = include_str!("prelude.js");

pub fn install<'js>(ctx: &Ctx<'js>) -> Result<()> {
    let native = Object::new(ctx.clone())?;
    native.set("print", Func::from(print))?;
    native.set("utf8Encode", Func::from(encoding::utf8_encode))?;
    native.set("utf8Decode", Func::from(encoding::utf8_decode))?;
    native.set("base64Encode", Func::from(bytes::base64_encode))?;
    native.set("base64Decode", Func::from(bytes::base64_decode))?;
    native.set("latin1Encode", Func::from(encoding::latin1_encode))?;
    native.set("latin1Decode", Func::from(encoding::latin1_decode))?;
    native.set("fillRandom", Func::from(fill_random))?;
    native.set("randomUUID", Func::from(random_uuid))?;
    native.set("now", Func::from(now))?;
    timers::install(ctx)?;
    ctx.globals().set("__demishell_native", native)?;
    ctx.eval::<(), _>(PRELUDE)?;
    Ok(())
}

/// Synchronous write of console output to a standard stream.
fn print(fd: i32, text: String) {
    let bytes = text.as_bytes();
    let mut done = 0;
    while done < bytes.len() {
        // SAFETY: plain write(2) on a descriptor the process owns.
        let n = unsafe { libc::write(fd, bytes[done..].as_ptr() as *const _, bytes.len() - done) };
        if n <= 0 {
            return;
        }
        done += n as usize;
    }
}

fn fill_random(ctx: Ctx<'_>, target: TypedArray<'_, u8>) -> Result<()> {
    let raw = target
        .as_raw()
        .ok_or_else(|| crate::error::invalid(&ctx, "getRandomValues", "detached buffer"))?;
    // SAFETY: the raw view covers exactly the typed array's bytes.
    let slice = unsafe { std::slice::from_raw_parts_mut(raw.ptr.as_ptr(), raw.len) };
    getrandom::fill(slice).map_err(|e| crate::error::throw_code(&ctx, "EIO", &e.to_string(), "getrandom", None))
}

fn random_uuid(ctx: Ctx<'_>) -> Result<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| crate::error::throw_code(&ctx, "EIO", &e.to_string(), "getrandom", None))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = |i: usize| format!("{:02x}", b[i]);
    Ok(format!(
        "{}{}{}{}-{}{}-{}{}-{}{}-{}{}{}{}{}{}",
        h(0), h(1), h(2), h(3), h(4), h(5), h(6), h(7), h(8), h(9), h(10), h(11), h(12), h(13), h(14), h(15)
    ))
}

fn now(ctx: Ctx<'_>) -> f64 {
    state(&ctx).start.elapsed().as_secs_f64() * 1000.0
}
