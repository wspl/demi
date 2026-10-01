// This file is part of the uutils coreutils package.
// See the distributed LICENSE for copyright and license information.

use std::path::PathBuf;
use tempfile::TempDir;
use uucore::context::fs::File;
use uucore::error::UResult;
use crate::SortError;

/// Owns one invocation's temporary directory and removes it on drop.
/// The runner owns the enclosing job directory for forced cancellation cleanup.
pub struct TmpDirWrapper {
    temp_dir: Option<TempDir>,
    parent_path: PathBuf,
    size: usize,
}

impl TmpDirWrapper {
    pub fn new(path: PathBuf) -> Self {
        Self {
            parent_path: uucore::context::resolve(&path),
            size: 0,
            temp_dir: None,
        }
    }

    pub fn next_file(&mut self) -> UResult<(File, PathBuf)> {
        if self.temp_dir.is_none() {
            self.temp_dir = Some(
                tempfile::Builder::new()
                    .prefix("uutils_sort")
                    .tempdir_in(&self.parent_path)
                    .map_err(|_| SortError::TmpFileCreationFailed {
                        path: self.parent_path.clone(),
                    })?,
            );
        }
        let path = self.temp_dir.as_ref().unwrap().path().join(self.size.to_string());
        self.size += 1;
        Ok((
            File::create(&path).map_err(|error| SortError::OpenTmpFileFailed { error })?,
            path,
        ))
    }
}

impl Drop for TmpDirWrapper {
    fn drop(&mut self) {
        if let Some(directory) = self.temp_dir.take() {
            if let Err(error) = directory.close() {
                uucore::show_error!("failed to remove sort temporary directory: {error}");
            }
        }
    }
}
