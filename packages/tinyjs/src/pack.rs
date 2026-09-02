//! Packing: what `tinyjsc` does. Compiles a bundle to bytecode with this
//! release's interpreter and injects it into a bare tinyjs of the same
//! release, for any platform, the way `bun build --compile` and
//! `deno compile` produce a deliverable.

use std::io;

use rquickjs::module::WriteOptions;
use rquickjs::{CatchResultExt, Context, Module, Runtime};

use crate::loader::{ShellLoader, ShellResolver};
use crate::payload::{encode_section, magic, own_release, release_of, Payload, ENTRY, SECTION};

/// Compiles the bundle to bytecode. Declaring a module resolves its import
/// graph, so the real loader is installed with a payload that has no
/// modules beside the entry: `tinyjs:*` declare (not evaluate) and any other
/// import fails here — a bundle is one file, and a packed binary could not
/// serve a second one.
fn compile(bundle_path: &str) -> io::Result<Vec<u8>> {
    let source = std::fs::read(bundle_path)?;
    let rt = Runtime::new().map_err(|e| io::Error::other(e.to_string()))?;
    rt.set_loader(ShellResolver, ShellLoader::new(Payload::Packed { bytecode: &[] }));
    let ctx = Context::full(&rt).map_err(|e| io::Error::other(e.to_string()))?;
    ctx.with(|ctx| {
        let module = Module::declare(ctx.clone(), ENTRY, source).catch(&ctx).map_err(|e| io::Error::other(format!("{e}")))?;
        module.write(WriteOptions::default()).catch(&ctx).map_err(|e| io::Error::other(format!("{e}")))
    })
}

/// The injected segment load command (a segment with one section) needs
/// 152 bytes of room between the last load command and the first section;
/// without it the injection would overwrite the start of `__text`. tinyjs
/// is linked with `-headerpad` for this, and the given binary is checked.
fn check_macho_headerpad(exe: &[u8]) -> io::Result<()> {
    const HEADER: usize = 32;
    const LC_SEGMENT_64: u32 = 0x19;
    let u32_at = |o: usize| u32::from_le_bytes(exe[o..o + 4].try_into().unwrap());
    let ncmds = u32_at(16) as usize;
    let sizeofcmds = u32_at(20) as usize;
    let header_end = HEADER + sizeofcmds;
    let mut first_section = usize::MAX;
    let mut pos = HEADER;
    for _ in 0..ncmds {
        let (cmd, cmdsize) = (u32_at(pos), u32_at(pos + 4) as usize);
        if cmd == LC_SEGMENT_64 {
            let nsects = u32_at(pos + 64) as usize;
            for i in 0..nsects {
                let sect = pos + 72 + i * 80;
                let offset = u32_at(sect + 48) as usize;
                let flags = u32_at(sect + 64);
                // Zero-fill sections have no file offset.
                if flags & 0xff != 0x1 && offset != 0 {
                    first_section = first_section.min(offset);
                }
            }
        }
        pos += cmdsize;
    }
    if first_section.saturating_sub(header_end) < 152 {
        return Err(io::Error::other("not enough Mach-O header padding to add a segment; link with -headerpad"));
    }
    Ok(())
}

/// Writes `out_path`: a copy of the bare tinyjs at `bin_path` with the
/// bundle compiled to bytecode and injected. The bare binary must be this
/// release and must not be packed already.
pub fn pack(bundle_path: &str, bin_path: &str, out_path: &str) -> io::Result<()> {
    let exe = std::fs::read(bin_path)?;
    let (release, abi) = release_of(&exe)?;
    let (own, own_abi) = own_release();
    if release != own || abi != own_abi {
        return Err(io::Error::other(format!("{bin_path} is tinyjs {release} (abi {abi}); this is tinyjsc {own} (abi {own_abi})")));
    }
    if has_section(&exe) {
        return Err(io::Error::other(format!("{bin_path} is already packed; pack a bare tinyjs")));
    }
    let section = encode_section(&compile(bundle_path)?);
    let mut out = std::fs::File::create(out_path)?;
    let fail = |e: libsui::Error| io::Error::other(e.to_string());
    if libsui::utils::is_macho(&exe) {
        check_macho_headerpad(&exe)?;
        libsui::Macho::from(exe).map_err(fail)?.write_section(SECTION, section).map_err(fail)?.build_and_sign(&mut out).map_err(fail)?;
    } else if libsui::utils::is_elf(&exe) {
        libsui::Elf::new(&exe).append(SECTION, &section, &mut out).map_err(fail)?;
    } else {
        return Err(io::Error::other("unsupported executable format"));
    }
    drop(out);
    std::fs::set_permissions(out_path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
}

/// Whether the image already carries a bundle section: its magic appears
/// forwards nowhere else in a tinyjs binary.
fn has_section(exe: &[u8]) -> bool {
    let magic = magic();
    exe.windows(magic.len()).any(|w| w == magic)
}
