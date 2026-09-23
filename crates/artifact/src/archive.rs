//! Archive extraction on the blocking pool.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use tokio_util::sync::CancellationToken;

use crate::Error;

/// zip checks each entry's path and CRC but has no cancellation of its own;
/// the reader it reads through stops when cancelled.
struct Cancellable {
    file: File,
    cancel: CancellationToken,
}

impl Read for Cancellable {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.is_cancelled() {
            // Not `Interrupted`: the standard library's readers retry that
            // kind, so extraction would spin instead of stopping.
            return Err(std::io::Error::other("extraction cancelled"));
        }
        self.file.read(buffer)
    }
}

impl Seek for Cancellable {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(position)
    }
}

/// Extracts the zip archive at `archive` into `destination`. The call returns
/// only once extraction has stopped, also when cancelled, so the caller alone
/// owns what was extracted.
pub async fn extract_zip(archive: &Path, destination: &Path, cancel: &CancellationToken) -> Result<(), Error> {
    let archive = archive.to_owned();
    let destination = destination.to_owned();
    let reader_cancel = cancel.clone();
    let extracted = tokio::task::spawn_blocking(move || {
        let reader = Cancellable {
            file: File::open(archive)?,
            cancel: reader_cancel,
        };
        let mut archive =
            zip::ZipArchive::new(reader).map_err(|error| Error::Archive(error.to_string()))?;
        archive
            .extract(destination)
            .map_err(|error| Error::Archive(error.to_string()))
    })
    .await
    .map_err(std::io::Error::other)?;
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    extracted
}
