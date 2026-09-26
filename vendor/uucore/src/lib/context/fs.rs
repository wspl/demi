//! Filesystem calls resolve relative paths against the utility's cwd.

pub use std::fs::{DirEntry, FileTimes, FileType, Metadata, Permissions, ReadDir};
use std::{
    io::{self, Read, Seek, SeekFrom, Write},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct File(std::fs::File);
impl File {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        OpenOptions::new().read(true).open(path)
    }
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        OpenOptions::new().write(true).create(true).truncate(true).open(path)
    }
    pub fn create_new(path: impl AsRef<Path>) -> io::Result<Self> {
        OpenOptions::new().write(true).create_new(true).open(path)
    }
    pub fn options() -> OpenOptions {
        OpenOptions::new()
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        super::duplicate(&self.0).map(Self)
    }
    pub fn set_len(&self, size: u64) -> io::Result<()> {
        super::check_cancelled();
        let _edit = super::control().and_then(|control| control.edit_file(&self.0));
        self.0.set_len(size)
    }
}
impl Deref for File {
    type Target = std::fs::File;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for File {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl From<std::fs::File> for File {
    fn from(file: std::fs::File) -> Self {
        Self(file)
    }
}
impl From<File> for std::fs::File {
    fn from(file: File) -> Self {
        file.0
    }
}
impl From<File> for std::process::Stdio {
    fn from(file: File) -> Self {
        file.0.into()
    }
}
impl Read for File {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        super::read(&self.0, bytes)
    }
}
impl Read for &File {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        super::read(&self.0, bytes)
    }
}
impl Write for File {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        super::write(&self.0, bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
impl Write for &File {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        super::write(&self.0, bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        (&self.0).flush()
    }
}
impl Seek for File {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.seek(position)
    }
}
impl Seek for &File {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        (&self.0).seek(position)
    }
}

#[derive(Clone, Debug)]
pub struct OpenOptions(std::fs::OpenOptions, u8);
impl Default for OpenOptions {
    fn default() -> Self {
        Self::new()
    }
}
impl OpenOptions {
    pub fn new() -> Self {
        Self(std::fs::OpenOptions::new(), 0)
    }
    pub fn read(&mut self, enabled: bool) -> &mut Self {
        self.0.read(enabled);
        self
    }
    pub fn write(&mut self, enabled: bool) -> &mut Self {
        self.1 = (self.1 & !(1 << 0)) | (u8::from(enabled) << 0);
        self.0.write(enabled);
        self
    }
    pub fn append(&mut self, enabled: bool) -> &mut Self {
        self.1 = (self.1 & !(1 << 1)) | (u8::from(enabled) << 1);
        self.0.append(enabled);
        self
    }
    pub fn truncate(&mut self, enabled: bool) -> &mut Self {
        self.1 = (self.1 & !(1 << 2)) | (u8::from(enabled) << 2);
        self.0.truncate(enabled);
        self
    }
    pub fn create(&mut self, enabled: bool) -> &mut Self {
        self.1 = (self.1 & !(1 << 3)) | (u8::from(enabled) << 3);
        self.0.create(enabled);
        self
    }
    pub fn create_new(&mut self, enabled: bool) -> &mut Self {
        self.1 = (self.1 & !(1 << 4)) | (u8::from(enabled) << 4);
        self.0.create_new(enabled);
        self
    }
    pub fn open(&self, path: impl AsRef<Path>) -> io::Result<File> {
        super::check_cancelled();
        if let Some(file) = super::descriptor(path.as_ref()) {
            return super::duplicate(&file).map(File);
        }
        let path = super::resolve(path);
        match super::control() {
            Some(control) => control.open(&path, &self.0, self.1 != 0).map(File),
            None => self.0.open(path).map(File),
        }
    }
}

pub struct DirBuilder(std::fs::DirBuilder);
impl Default for DirBuilder {
    fn default() -> Self {
        Self::new()
    }
}
impl DirBuilder {
    pub fn new() -> Self {
        Self(std::fs::DirBuilder::new())
    }
    pub fn recursive(&mut self, enabled: bool) -> &mut Self {
        self.0.recursive(enabled);
        self
    }
    pub fn create(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.0.create(super::resolve(path))
    }
}

pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    super::check_cancelled();
    if let Some(file) = super::descriptor(path.as_ref()) {
        return file.metadata();
    }
    std::fs::metadata(super::resolve(path))
}
pub fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    std::fs::symlink_metadata(super::resolve(path))
}
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let mut text = String::new();
    File::open(path)?.read_to_string(&mut text)?;
    Ok(text)
}
pub fn read_dir(path: impl AsRef<Path>) -> io::Result<ReadDir> {
    std::fs::read_dir(super::resolve(path))
}
pub fn read_link(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::fs::read_link(super::resolve(path))
}
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::fs::canonicalize(super::resolve(path))
}
pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    let path = super::resolve(path);
    let _edit = super::edit(&path);
    std::fs::write(path, bytes)
}
pub fn create_dir(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::create_dir(super::resolve(path))
}
pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::create_dir_all(super::resolve(path))
}
pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::remove_file(super::resolve(path))
}
pub fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::remove_dir(super::resolve(path))
}
pub fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::remove_dir_all(super::resolve(path))
}
pub fn set_permissions(path: impl AsRef<Path>, permissions: Permissions) -> io::Result<()> {
    std::fs::set_permissions(super::resolve(path), permissions)
}
pub fn rename(source: impl AsRef<Path>, destination: impl AsRef<Path>) -> io::Result<()> {
    std::fs::rename(super::resolve(source), super::resolve(destination))
}
pub fn copy(source: impl AsRef<Path>, destination: impl AsRef<Path>) -> io::Result<u64> {
    std::fs::copy(super::resolve(source), super::resolve(destination))
}
pub fn hard_link(source: impl AsRef<Path>, destination: impl AsRef<Path>) -> io::Result<()> {
    std::fs::hard_link(super::resolve(source), super::resolve(destination))
}
pub fn exists(path: impl AsRef<Path>) -> io::Result<bool> {
    std::fs::exists(super::resolve(path))
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::fd::*;
    use std::os::unix::fs::{DirBuilderExt, FileExt, OpenOptionsExt};
    impl AsFd for File {
        fn as_fd(&self) -> BorrowedFd<'_> {
            self.0.as_fd()
        }
    }
    impl AsRawFd for File {
        fn as_raw_fd(&self) -> RawFd {
            self.0.as_raw_fd()
        }
    }
    impl IntoRawFd for File {
        fn into_raw_fd(self) -> RawFd {
            self.0.into_raw_fd()
        }
    }
    impl FromRawFd for File {
        unsafe fn from_raw_fd(fd: RawFd) -> Self {
            Self(unsafe { std::fs::File::from_raw_fd(fd) })
        }
    }
    impl From<OwnedFd> for File {
        fn from(fd: OwnedFd) -> Self {
            Self(fd.into())
        }
    }
    impl From<File> for OwnedFd {
        fn from(file: File) -> Self {
            file.0.into()
        }
    }
    impl OpenOptionsExt for OpenOptions {
        fn mode(&mut self, mode: u32) -> &mut Self {
            self.0.mode(mode);
            self
        }
        fn custom_flags(&mut self, flags: i32) -> &mut Self {
            self.0.custom_flags(flags);
            self
        }
    }
    impl DirBuilderExt for DirBuilder {
        fn mode(&mut self, mode: u32) -> &mut Self {
            self.0.mode(mode);
            self
        }
    }
    impl FileExt for File {
        fn read_at(&self, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
            self.0.read_at(bytes, offset)
        }
        fn write_at(&self, bytes: &[u8], offset: u64) -> io::Result<usize> {
            let _edit = super::super::control().and_then(|control| control.edit_file(&self.0));
            self.0.write_at(bytes, offset)
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::{
        fs::{FileExt, OpenOptionsExt},
        io::*,
    };
    impl AsHandle for File {
        fn as_handle(&self) -> BorrowedHandle<'_> {
            self.0.as_handle()
        }
    }
    impl AsRawHandle for File {
        fn as_raw_handle(&self) -> RawHandle {
            self.0.as_raw_handle()
        }
    }
    impl IntoRawHandle for File {
        fn into_raw_handle(self) -> RawHandle {
            self.0.into_raw_handle()
        }
    }
    impl FromRawHandle for File {
        unsafe fn from_raw_handle(handle: RawHandle) -> Self {
            Self(unsafe { std::fs::File::from_raw_handle(handle) })
        }
    }
    impl From<OwnedHandle> for File {
        fn from(handle: OwnedHandle) -> Self {
            Self(handle.into())
        }
    }
    impl From<File> for OwnedHandle {
        fn from(file: File) -> Self {
            file.0.into()
        }
    }
    impl OpenOptionsExt for OpenOptions {
        fn access_mode(&mut self, value: u32) -> &mut Self {
            self.0.access_mode(value);
            self
        }
        fn share_mode(&mut self, value: u32) -> &mut Self {
            self.0.share_mode(value);
            self
        }
        fn custom_flags(&mut self, value: u32) -> &mut Self {
            self.0.custom_flags(value);
            self
        }
        fn attributes(&mut self, value: u32) -> &mut Self {
            self.0.attributes(value);
            self
        }
        fn security_qos_flags(&mut self, value: u32) -> &mut Self {
            self.0.security_qos_flags(value);
            self
        }
    }
    impl FileExt for File {
        fn seek_read(&self, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
            self.0.seek_read(bytes, offset)
        }
        fn seek_write(&self, bytes: &[u8], offset: u64) -> io::Result<usize> {
            let _edit = super::super::control().and_then(|control| control.edit_file(&self.0));
            self.0.seek_write(bytes, offset)
        }
    }
}

#[cfg(unix)]
pub fn symlink(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    std::os::unix::fs::symlink(target, super::resolve(link))
}
#[cfg(windows)]
pub fn symlink_file(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, super::resolve(link))
}
#[cfg(windows)]
pub fn symlink_dir(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(target, super::resolve(link))
}
