// This file is part of the uutils coreutils package.
//
// For the full copyright and license information, please view the LICENSE
// file that was distributed with this source code.

//! What `stat` shows of a Windows file, by the names its Unix directives
//! read: the volume's serial number as the device, the file index as the
//! inode, the owner's and group's relative IDs as the user and group IDs,
//! and the mode `ls -l` shows (`uucore::fs::portable_mode`). Windows has no
//! special files, so a file's device type is 0.

use std::cell::OnceCell;
use std::ops::Deref;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use uucore::context::fs::{FileType, Metadata};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    GROUP_SECURITY_INFORMATION, GetSidSubAuthority, GetSidSubAuthorityCount, IsValidSid,
    LookupAccountSidW, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_STANDARD_INFO, FileBasicInfo, FileStandardInfo,
    GetFileInformationByHandle, GetFileInformationByHandleEx,
};

/// A file's metadata with what Windows keeps beside it.
pub struct Stat {
    metadata: Metadata,
    path: PathBuf,
    identity: BY_HANDLE_FILE_INFORMATION,
    basic: FILE_BASIC_INFO,
    standard: FILE_STANDARD_INFO,
    /// Read only when a directive asks for them.
    owners: OnceCell<Owners>,
}

/// A file's owner and group, by relative ID and account name; none when
/// the file's security cannot be read.
#[derive(Default)]
struct Owners {
    user: Option<Account>,
    group: Option<Account>,
}

struct Account {
    id: u32,
    name: Option<String>,
}

impl Stat {
    /// The file `path` names, or with `follow` the file a link there points
    /// to, whose `metadata` the caller read.
    pub fn new(path: &Path, follow: bool, metadata: Metadata) -> std::io::Result<Self> {
        let path = uucore::context::resolve(path);
        let flags = FILE_FLAG_BACKUP_SEMANTICS | if follow { 0 } else { FILE_FLAG_OPEN_REPARSE_POINT };
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(flags)
            .open(&path)?;
        let handle = file.as_raw_handle();
        // SAFETY: each structure is plain data, which zeroes make valid, and
        // the calls below fill.
        let (mut identity, mut basic, mut standard): (
            BY_HANDLE_FILE_INFORMATION,
            FILE_BASIC_INFO,
            FILE_STANDARD_INFO,
        ) = unsafe { (std::mem::zeroed(), std::mem::zeroed(), std::mem::zeroed()) };
        // SAFETY: `handle` is open for the calls, and each buffer is the
        // size the call is told.
        let read = unsafe {
            GetFileInformationByHandle(handle, &raw mut identity) != 0
                && GetFileInformationByHandleEx(
                    handle,
                    FileBasicInfo,
                    (&raw mut basic).cast(),
                    size_of::<FILE_BASIC_INFO>() as u32,
                ) != 0
                && GetFileInformationByHandleEx(
                    handle,
                    FileStandardInfo,
                    (&raw mut standard).cast(),
                    size_of::<FILE_STANDARD_INFO>() as u32,
                ) != 0
        };
        if !read {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self {
            metadata,
            path,
            identity,
            basic,
            standard,
            owners: OnceCell::new(),
        })
    }

    pub fn mode(&self) -> u32 {
        uucore::fs::portable_mode(&self.metadata)
    }

    pub fn dev(&self) -> u64 {
        u64::from(self.identity.dwVolumeSerialNumber)
    }

    pub fn ino(&self) -> u64 {
        (u64::from(self.identity.nFileIndexHigh) << 32) | u64::from(self.identity.nFileIndexLow)
    }

    pub fn nlink(&self) -> u64 {
        u64::from(self.identity.nNumberOfLinks)
    }

    /// The 512-byte blocks allocated to the file.
    pub fn blocks(&self) -> u64 {
        self.standard.AllocationSize.max(0).cast_unsigned().div_ceil(512)
    }

    pub fn blksize(&self) -> u64 {
        4096
    }

    pub fn rdev(&self) -> u64 {
        0
    }

    /// The last change of the file's data or attributes.
    pub fn change_time(&self) -> Option<SystemTime> {
        // A FILETIME counts 100-nanosecond intervals from 1601.
        const FROM_1601: u64 = 11_644_473_600;
        let ticks = u64::try_from(self.basic.ChangeTime).ok()?;
        let since_1601 = Duration::from_nanos(ticks.checked_mul(100)?);
        since_1601
            .checked_sub(Duration::from_secs(FROM_1601))
            .map(|since_1970| UNIX_EPOCH + since_1970)
    }

    pub fn uid(&self) -> u32 {
        self.owners().user.as_ref().map_or(0, |account| account.id)
    }

    pub fn gid(&self) -> u32 {
        self.owners().group.as_ref().map_or(0, |account| account.id)
    }

    pub fn user_name(&self) -> Option<String> {
        self.owners().user.as_ref()?.name.clone()
    }

    pub fn group_name(&self) -> Option<String> {
        self.owners().group.as_ref()?.name.clone()
    }

    fn owners(&self) -> &Owners {
        self.owners.get_or_init(|| owners(&self.path))
    }
}

impl Deref for Stat {
    type Target = Metadata;

    fn deref(&self) -> &Metadata {
        &self.metadata
    }
}

/// Whether the file is a device; Windows has none a path names.
pub fn is_device(_file_type: FileType) -> bool {
    false
}

/// The major and minor parts of a device number as Linux packs them.
pub fn major(dev: u64) -> u64 {
    ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff)
}

pub fn minor(dev: u64) -> u64 {
    (dev & 0xff) | ((dev >> 12) & !0xff)
}

/// The owner and group of the file at `path`.
fn owners(path: &Path) -> Owners {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let (mut user, mut group, mut descriptor): (PSID, PSID, PSECURITY_DESCRIPTOR) =
        (std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
    // SAFETY: `wide` is nul-terminated; the call allocates the descriptor,
    // which the SIDs point into and which is freed below.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION,
            &raw mut user,
            &raw mut group,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Owners::default();
    }
    let owners = Owners {
        user: account(user),
        group: account(group),
    };
    // SAFETY: the descriptor came from GetNamedSecurityInfoW, which leaves
    // freeing it to the caller; nothing points into it any more.
    unsafe { LocalFree(descriptor) };
    owners
}

/// The account a SID names: its relative ID, the last part of the SID, as
/// Cygwin numbers a local account, and its name, if the system knows it.
fn account(sid: PSID) -> Option<Account> {
    // SAFETY: `sid` points into a live security descriptor; IsValidSid
    // vouches for the reads after it.
    unsafe {
        if sid.is_null() || IsValidSid(sid) == 0 {
            return None;
        }
        let count = *GetSidSubAuthorityCount(sid);
        let id = if count == 0 {
            0
        } else {
            *GetSidSubAuthority(sid, u32::from(count) - 1)
        };
        let (mut name, mut domain) = ([0u16; 256], [0u16; 256]);
        let (mut name_length, mut domain_length, mut kind) =
            (name.len() as u32, domain.len() as u32, 0);
        let found = LookupAccountSidW(
            std::ptr::null(),
            sid,
            name.as_mut_ptr(),
            &raw mut name_length,
            domain.as_mut_ptr(),
            &raw mut domain_length,
            &raw mut kind,
        ) != 0;
        let name = found.then(|| String::from_utf16_lossy(&name[..name_length as usize]));
        Some(Account { id, name })
    }
}
