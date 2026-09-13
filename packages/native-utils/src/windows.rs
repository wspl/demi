//! Windows metadata presentation for the portable shell command set.

use std::{
    ffi::OsString,
    fs::Metadata,
    time::{SystemTime, UNIX_EPOCH},
};
use std::{
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
};
use uucore::context;
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_STANDARD_INFO, FileBasicInfo, FileStandardInfo,
    GetFileInformationByHandle, GetFileInformationByHandleEx,
};

struct FileInfo {
    identity: BY_HANDLE_FILE_INFORMATION,
    basic: FILE_BASIC_INFO,
    standard: FILE_STANDARD_INFO,
}

fn file_info(path: &Path, follow: bool) -> std::io::Result<FileInfo> {
    let flags = FILE_FLAG_BACKUP_SEMANTICS
        | if follow {
            0
        } else {
            FILE_FLAG_OPEN_REPARSE_POINT
        };
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(flags)
        .open(path)?;
    let mut info: FileInfo = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info.identity) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&mut info.basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileStandardInfo,
            (&mut info.standard as *mut FILE_STANDARD_INFO).cast(),
            std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(info)
}

pub fn stat(args: Vec<OsString>) -> i32 {
    match stat_inner(args) {
        Ok(()) => 0,
        Err(error) => {
            uucore::context_eprintln!("stat: {error}");
            1
        }
    }
}

fn stat_inner(args: Vec<OsString>) -> Result<(), String> {
    let mut args = args.into_iter().skip(1);
    let mut format = None;
    let mut newline = true;
    let mut follow = false;
    let mut operands = Vec::new();
    let mut options = true;
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        if !options || !text.starts_with('-') || text == "-" {
            operands.push(arg);
            continue;
        }
        match text.as_ref() {
            "--" => options = false,
            "-L" | "--dereference" => follow = true,
            "--help" => {
                uucore::context_println!(
                    "Usage: stat [-L] [-c FORMAT | --printf FORMAT] FILE...\nDisplay Windows file metadata. Ownership and mode use the portable shell representation."
                );
                return Ok(());
            }
            "-c" | "--format" | "--printf" => {
                newline = text != "--printf";
                format = Some(
                    args.next()
                        .ok_or("format argument is required")?
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            "-t" | "--terse" => format = Some("%n %s %b %f %u %g %d %i %h %X %Y %Z %W %o".into()),
            _ if text.starts_with("--format=") => format = Some(text[9..].to_owned()),
            _ if text.starts_with("--printf=") => {
                format = Some(text[9..].to_owned());
                newline = false;
            }
            _ => return Err(format!("unsupported option: {text}")),
        }
    }
    if operands.is_empty() {
        return Err("missing file operand".into());
    }
    let mut failed = false;
    for operand in operands {
        let path = context::resolve(&operand);
        let metadata = if follow {
            std::fs::metadata(&path)
        } else {
            std::fs::symlink_metadata(&path)
        };
        let metadata = match metadata {
            Ok(metadata) => metadata,
            Err(error) => {
                uucore::context_eprintln!("stat: {}: {error}", operand.to_string_lossy());
                failed = true;
                continue;
            }
        };
        let info = file_info(&path, follow).map_err(|error| error.to_string())?;
        let default = "  File: %n\n  Size: %s\tBlocks: %b\tIO Block: %o\t%F\nAccess: (%a)  Uid: (%u)  Gid: (%g)\nAccess: %x\nModify: %y\nChange: %z\n Birth: %w";
        let rendered = render(
            format.as_deref().unwrap_or(default),
            &operand.to_string_lossy(),
            &metadata,
            &info,
            !newline,
        )?;
        uucore::context_print!("{rendered}");
        if newline {
            uucore::context_println!();
        }
    }
    if failed {
        Err("one or more files could not be read".into())
    } else {
        Ok(())
    }
}

fn seconds(time: std::io::Result<SystemTime>) -> String {
    time.ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|time| time.as_secs().to_string())
        .unwrap_or_else(|| "0".into())
}

fn render(
    format: &str,
    name: &str,
    metadata: &Metadata,
    info: &FileInfo,
    escapes: bool,
) -> Result<String, String> {
    let mut result = String::new();
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if escapes && ch == '\\' {
            result.push(match chars.next().ok_or("incomplete format escape")? {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '0' => '\0',
                '\\' => '\\',
                other => return Err(format!("unsupported escape: {other}")),
            });
        } else if ch == '%' {
            let field = chars.next().ok_or("incomplete format directive")?;
            let value = match field {
                '%' => "%".into(),
                'n' => name.into(),
                'N' => format!("'{name}'"),
                's' => metadata.len().to_string(),
                'b' => (info.standard.AllocationSize.max(0) as u64)
                    .div_ceil(512)
                    .to_string(),
                'B' => "512".into(),
                'o' => "4096".into(),
                'F' => if metadata.is_dir() {
                    "directory"
                } else if metadata.is_symlink() {
                    "symbolic link"
                } else {
                    "regular file"
                }
                .into(),
                'a' => if metadata.is_dir() {
                    "755"
                } else if metadata.permissions().readonly() {
                    "444"
                } else {
                    "644"
                }
                .into(),
                'u' | 'g' => "0".into(),
                'd' => info.identity.dwVolumeSerialNumber.to_string(),
                'D' => format!("{:x}", info.identity.dwVolumeSerialNumber),
                'i' => (((info.identity.nFileIndexHigh as u64) << 32)
                    | info.identity.nFileIndexLow as u64)
                    .to_string(),
                'f' => format!(
                    "{:x}",
                    if metadata.is_dir() {
                        0o40755
                    } else if metadata.is_symlink() {
                        0o120777
                    } else if metadata.permissions().readonly() {
                        0o100444
                    } else {
                        0o100644
                    }
                ),
                'U' | 'G' => "UNKNOWN".into(),
                'h' => info.identity.nNumberOfLinks.to_string(),
                'X' => seconds(metadata.accessed()),
                'Y' => seconds(metadata.modified()),
                'W' => seconds(metadata.created()),
                'Z' => (info.basic.ChangeTime / 10_000_000 - 11_644_473_600).to_string(),
                'x' => timestamp(info.basic.LastAccessTime),
                'y' => timestamp(info.basic.LastWriteTime),
                'z' => timestamp(info.basic.ChangeTime),
                'w' => timestamp(info.basic.CreationTime),
                _ => return Err(format!("unsupported format directive: %{field}")),
            };
            result.push_str(&value);
        } else {
            result.push(ch);
        }
    }
    Ok(result)
}

fn timestamp(ticks: i64) -> String {
    let seconds = ticks.div_euclid(10_000_000) - 11_644_473_600;
    let nanos = (ticks.rem_euclid(10_000_000) * 100) as u32;
    chrono::DateTime::from_timestamp(seconds, nanos)
        .map(|time| time.format("%Y-%m-%d %H:%M:%S%.9f +0000").to_string())
        .unwrap_or_else(|| "-".into())
}
