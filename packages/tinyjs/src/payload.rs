//! Where the embedded bundle comes from.
//!
//! A packed binary carries the bundle as QuickJS bytecode in a section of
//! the executable, put there by `tinyjs --pack` through `libsui` (the
//! injector behind `deno compile`): a new segment on Mach-O, re-signed ad
//! hoc; a `PT_NOTE` on ELF. At start tinyjs asks `libsui::find_section`
//! for it in its own mapped image and loads it without parsing; nothing is
//! read from disk. The bare binary instead runs the entry file named on the
//! command line, with the entry's directory as the `/embedded/` namespace.

use std::io;

use rquickjs::module::WriteOptions;
use rquickjs::{CatchResultExt, Context, Module, Runtime};

pub const ENTRY: &str = "/embedded/main.mjs";
pub const SECTION: &str = "TINYJS_BUNDLE";

pub enum Payload {
    /// The bundle packed into the executable, as bytecode.
    Packed { bytecode: &'static [u8] },
    /// A directory on disk standing in for `/embedded/`.
    Directory { dir: String, entry: String },
}

/// The entry module as the loader receives it.
pub enum Entry {
    Bytecode(&'static [u8]),
    Source(Vec<u8>),
}

impl Payload {
    /// The name under which the entry module is evaluated.
    pub fn entry_name(&self) -> String {
        match self {
            Payload::Packed { .. } => ENTRY.to_string(),
            Payload::Directory { entry, .. } => entry.clone(),
        }
    }

    pub fn entry(&self) -> io::Result<Entry> {
        match self {
            Payload::Packed { bytecode } => Ok(Entry::Bytecode(bytecode)),
            Payload::Directory { dir, entry } => {
                let name = entry.strip_prefix("/embedded/").unwrap_or(entry);
                Ok(Entry::Source(std::fs::read(format!("{dir}/{name}"))?))
            }
        }
    }

    /// Resolves an `/embedded/…` import to its source. Only the directory
    /// payload has modules besides the entry.
    pub fn source(&self, name: &str) -> Option<io::Result<Vec<u8>>> {
        let rest = name.strip_prefix("/embedded/")?;
        match self {
            Payload::Packed { .. } => None,
            Payload::Directory { dir, .. } => Some(std::fs::read(format!("{dir}/{rest}"))),
        }
    }
}

/// The packed bundle, if the executable carries one.
pub fn packed() -> io::Result<Option<&'static [u8]>> {
    libsui::find_section(SECTION)
}

/// Maps a command-line entry path to the directory payload.
pub fn directory(entry_path: &str) -> io::Result<Payload> {
    let path = std::fs::canonicalize(entry_path)?;
    let dir = path.parent().unwrap_or(std::path::Path::new("/")).to_string_lossy().into_owned();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Payload::Directory { dir, entry: format!("/embedded/{name}") })
}

/// Compiles the bundle to bytecode with this tinyjs's QuickJS. Declaring a
/// module resolves its import graph, so the real loader is installed: the
/// `tinyjs:*` modules are declared (not evaluated) and a bundle importing
/// something that does not exist fails here rather than on a target. The
/// bytecode is tied to this interpreter build, which is why a bundle is
/// packed by the tinyjs release it will run on.
fn compile(bundle_path: &str) -> io::Result<Vec<u8>> {
    let source = std::fs::read(bundle_path)?;
    let rt = Runtime::new().map_err(|e| io::Error::other(e.to_string()))?;
    rt.set_loader(crate::loader::ShellResolver, crate::loader::ShellLoader::new(directory(bundle_path)?));
    let ctx = Context::full(&rt).map_err(|e| io::Error::other(e.to_string()))?;
    ctx.with(|ctx| {
        let module = Module::declare(ctx.clone(), ENTRY, source).catch(&ctx).map_err(|e| io::Error::other(format!("{e}")))?;
        module.write(WriteOptions::default()).catch(&ctx).map_err(|e| io::Error::other(format!("{e}")))
    })
}

/// The injected segment load command (a segment with one section) needs
/// 152 bytes of room between the last load command and the first section;
/// without it the injection would overwrite the start of `__text`. The
/// binary is linked with `-headerpad` for this, and a foreign `--bin` is
/// checked the same way.
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

/// `tinyjs --pack <bundle> --out <file> [--bin <bare tinyjs>]`: writes a
/// copy of a bare tinyjs (this executable by default; another platform's
/// build of the same release when cross-packing) with the bundle compiled
/// to bytecode and injected, the way `bun build --compile` and
/// `deno compile` produce a deliverable.
pub fn pack(bundle_path: &str, out_path: &str, bin_path: Option<&str>) -> io::Result<()> {
    let exe = match bin_path {
        Some(p) => std::fs::read(p)?,
        None => std::fs::read(std::env::current_exe()?)?,
    };
    if bin_path.is_none() && libsui::find_section(SECTION)?.is_some() {
        return Err(io::Error::other("this tinyjs is already packed; pack from the bare binary"));
    }
    let bytecode = compile(bundle_path)?;
    let mut out = std::fs::File::create(out_path)?;
    let fail = |e: libsui::Error| io::Error::other(e.to_string());
    if libsui::utils::is_macho(&exe) {
        check_macho_headerpad(&exe)?;
        libsui::Macho::from(exe).map_err(fail)?.write_section(SECTION, bytecode).map_err(fail)?.build_and_sign(&mut out).map_err(fail)?;
    } else if libsui::utils::is_elf(&exe) {
        libsui::Elf::new(&exe).append(SECTION, &bytecode, &mut out).map_err(fail)?;
    } else {
        return Err(io::Error::other("unsupported executable format"));
    }
    drop(out);
    std::fs::set_permissions(out_path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
}
