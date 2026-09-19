//! Files a viewer chooses for a file input (`browser-live-view.md` § Input).
//! Their bytes travel through the stream itself into a directory of the
//! environment, then attach to the input the viewer saw.

use std::{path::PathBuf, sync::Arc};

use bytes::Bytes;
use tokio::{io::AsyncWriteExt, sync::mpsc};
use tokio_util::task::AbortOnDropHandle;

use super::{
    super::protocol::{LiveInboundUploadFilesItem, LiveOutbound},
    commands::find,
    hub::Membership,
    observers,
    writer::Writer,
};
use crate::browser::{BrowserEnvironment, BrowserError, Result, handles};

pub(super) enum Item {
    Start {
        tab: String,
        token: String,
        revision: u64,
        upload: u64,
        files: Vec<LiveInboundUploadFilesItem>,
    },
    Data {
        upload: u32,
        file: u32,
        data: Bytes,
    },
}

struct File {
    path: PathBuf,
    size: u64,
    written: u64,
    handle: Option<tokio::fs::File>,
}

struct Upload {
    id: u64,
    tab: String,
    token: String,
    revision: u64,
    files: Vec<File>,
    /// The file whose bytes arrive next.
    current: usize,
}

impl Upload {
    /// Skips files that are complete, including empty ones.
    async fn advance(&mut self) -> Result<()> {
        while let Some(file) = self.files.get_mut(self.current)
            && file.written == file.size
        {
            if let Some(mut handle) = file.handle.take() {
                handle.flush().await?;
            }
            self.current += 1;
        }
        Ok(())
    }

    fn complete(&self) -> bool {
        self.current == self.files.len()
    }
}

pub(super) fn start(
    environment: BrowserEnvironment,
    membership: Arc<Membership>,
    writer: Writer,
    mut items: mpsc::Receiver<Item>,
) -> AbortOnDropHandle<()> {
    AbortOnDropHandle::new(tokio::spawn(async move {
        let mut pending: Option<Upload> = None;
        while let Some(item) = items.recv().await {
            let outcome = match item {
                Item::Start {
                    tab,
                    token,
                    revision,
                    upload,
                    files,
                } => {
                    membership.operated();
                    match prepare(&environment, upload, tab, token.clone(), revision, files).await {
                        Ok(upload) => {
                            pending = Some(upload);
                            Ok(())
                        }
                        Err(error) => {
                            pending = None;
                            refuse(&writer, token, &error).await;
                            continue;
                        }
                    }
                }
                Item::Data { upload, file, data } => {
                    let Some(current) = pending.as_mut() else {
                        continue;
                    };
                    // Bytes of an upload already refused or replaced.
                    if current.id != u64::from(upload) {
                        continue;
                    }
                    receive(current, file, data).await
                }
            };
            let Some(current) = pending.as_mut() else {
                continue;
            };
            let outcome = match outcome {
                Ok(()) => current.advance().await,
                Err(error) => Err(error),
            };
            if let Err(error) = outcome {
                let refused = pending.take().expect("pending upload");
                refuse(&writer, refused.token, &error).await;
                continue;
            }
            if current.complete() {
                let done = pending.take().expect("pending upload");
                let paths = done
                    .files
                    .iter()
                    .map(|file| file.path.to_string_lossy().into_owned())
                    .collect();
                let accepted = async {
                    let tab = find(&environment, &done.tab).await?;
                    observers::attach(&tab, &done.token, done.revision, paths).await
                }
                .await;
                match accepted {
                    Ok(accepted) => {
                        writer
                            .control(&LiveOutbound::Choice {
                                token: done.token,
                                accepted,
                            })
                            .await;
                    }
                    Err(error) => refuse(&writer, done.token, &error).await,
                }
            }
        }
    }))
}

async fn prepare(
    environment: &BrowserEnvironment,
    id: u64,
    tab: String,
    token: String,
    revision: u64,
    files: Vec<LiveInboundUploadFilesItem>,
) -> Result<Upload> {
    let directory = environment.upload_directory.join(handles::fresh("u")?);
    tokio::fs::create_dir(&directory).await?;
    let mut prepared = Vec::with_capacity(files.len());
    for file in files {
        // The protocol already excludes separators; a name must also stay a
        // name, and one file must not replace another.
        if matches!(file.name.as_str(), "." | "..")
            || prepared
                .iter()
                .any(|existing: &File| existing.path.file_name() == Some(file.name.as_ref()))
        {
            return Err(BrowserError::Configuration(format!(
                "invalid file name: {}",
                file.name
            )));
        }
        let path = directory.join(&file.name);
        let handle = tokio::fs::File::create(&path).await?;
        prepared.push(File {
            path,
            size: file.size,
            written: 0,
            handle: Some(handle),
        });
    }
    let mut upload = Upload {
        id,
        tab,
        token,
        revision,
        files: prepared,
        current: 0,
    };
    upload.advance().await?;
    Ok(upload)
}

async fn receive(upload: &mut Upload, file: u32, data: Bytes) -> Result<()> {
    let index = usize::try_from(file).unwrap_or(usize::MAX);
    let current = upload.current;
    let Some(entry) = upload.files.get_mut(current).filter(|_| index == current) else {
        return Err(BrowserError::Configuration(
            "file bytes arrived out of order".into(),
        ));
    };
    let written = entry.written + data.len() as u64;
    if written > entry.size {
        return Err(BrowserError::Configuration(
            "a file is larger than announced".into(),
        ));
    }
    if let Some(handle) = entry.handle.as_mut() {
        handle.write_all(&data).await?;
    }
    entry.written = written;
    Ok(())
}

async fn refuse(writer: &Writer, token: String, error: &BrowserError) {
    writer.notice(error.code(), &error.to_string()).await;
    writer
        .control(&LiveOutbound::Choice {
            token,
            accepted: false,
        })
        .await;
}
