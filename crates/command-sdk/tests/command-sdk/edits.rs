//! The invocation edit recorder (`crates/command-sdk/src/edits.rs`): what it records of a
//! job's file changes, and what it leaves out.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::Path,
};

use demi_command_protocol::{EDIT_FILE_BYTES, EditContext, EditCopies, EditKind};
use demi_command_sdk::edits::Recorder;

fn recorder(root: &Path, job: &str) -> Recorder {
    Recorder::new(EditContext {
        directory: root.join(job).to_string_lossy().into_owned(),
        lock: root.join("edits.lock").to_string_lossy().into_owned(),
    })
    .unwrap()
}

fn contents(edit: &EditCopies) -> (String, String) {
    (
        edit.original
            .as_ref()
            .map(|path| fs::read_to_string(path).unwrap())
            .unwrap_or_default(),
        fs::read_to_string(edit.modified.as_ref().unwrap()).unwrap(),
    )
}

#[test]
fn separate_recorder_handles_share_the_job_journal() {
    let root = tempfile::tempdir().unwrap();
    let first = recorder(root.path(), "job");
    let native = recorder(root.path(), "job");
    let path = root.path().join("file");
    first.record(&path, || fs::write(&path, "one")).unwrap();
    native.record(&path, || fs::write(&path, "two")).unwrap();
    let report = first.report().unwrap();
    assert_eq!(report.files[0].kind, EditKind::Added);
    assert_eq!(report.files[0].edits.len(), 1);
    assert_eq!(
        contents(&report.files[0].edits[0]),
        ("".into(), "two".into())
    );
}

#[test]
fn an_error_can_leave_a_real_edit() {
    let root = tempfile::tempdir().unwrap();
    let recorder = recorder(root.path(), "job");
    let path = root.path().join("file");
    let result: io::Result<()> = recorder.record(&path, || {
        fs::write(&path, "partial")?;
        Err(io::Error::other("failed after writing"))
    });
    assert!(result.is_err());
    assert_eq!(
        contents(&recorder.report().unwrap().files[0].edits[0]).1,
        "partial"
    );
}

#[test]
fn failed_open_of_large_file_is_not_an_edit() {
    let root = tempfile::tempdir().unwrap();
    let recorder = recorder(root.path(), "job");
    let path = root.path().join("large");
    File::create(&path)
        .unwrap()
        .set_len(EDIT_FILE_BYTES as u64 + 1)
        .unwrap();
    let result = recorder.record(&path, || File::create_new(&path));
    assert!(result.is_err());
    assert!(recorder.report().unwrap().files.is_empty());
    recorder
        .record(&path, || {
            OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(b"x")
        })
        .unwrap();
    let report = recorder.report().unwrap();
    assert_eq!(report.files.len(), 1);
    assert!(report.files[0].edits[0].modified.is_none());
}

#[test]
fn a_failed_write_below_a_file_is_not_an_edit() {
    let root = tempfile::tempdir().unwrap();
    let recorder = recorder(root.path(), "job");
    let file = root.path().join("file");
    fs::write(&file, "plain").unwrap();
    let below = file.join("child");
    let result = recorder.record(&below, || fs::write(&below, "never"));
    assert!(result.is_err());
    assert!(recorder.report().unwrap().files.is_empty());
}

/// Binary is what git calls binary
/// (`crates/command-package-file/src/files.rs`).
///
/// A NUL in the first 8 KiB leaves an edit without contents, marked binary for
/// the Change view to say why. So does a file over the limit whose first bytes
/// are binary. A Latin-1 file is text whose copy is kept as written, for the
/// change view to show its odd bytes as U+FFFD.
///
/// Planted defect this catches: a binary edit not marked, so the view cannot
/// tell it from one over the limits.
#[test]
fn only_binary_edits_have_no_contents() {
    let root = tempfile::tempdir().unwrap();
    let recorder = recorder(root.path(), "job");
    let binary = root.path().join("binary");
    recorder
        .record(&binary, || fs::write(&binary, [b'a', 0, 1, 2]))
        .unwrap();
    let latin1 = root.path().join("latin1.txt");
    recorder
        .record(&latin1, || fs::write(&latin1, b"caf\xe9\n"))
        .unwrap();
    let archive = root.path().join("archive");
    recorder
        .record(&archive, || {
            let mut bytes = b"PK\x03\x04\0\0".to_vec();
            bytes.resize(EDIT_FILE_BYTES + 1, 7);
            fs::write(&archive, bytes)
        })
        .unwrap();
    let edit = |path: &Path| {
        let report = recorder.report().unwrap();
        let file = report
            .files
            .into_iter()
            .find(|file| Path::new(&file.path).file_name() == path.file_name())
            .unwrap();
        assert_eq!(file.kind, EditKind::Added);
        file.edits[0].clone()
    };
    let log = root.path().join("big.log");
    recorder
        .record(&log, || fs::write(&log, "x".repeat(EDIT_FILE_BYTES + 1)))
        .unwrap();
    let too_large = edit(&log);
    assert_eq!((too_large.modified, too_large.binary), (None, false));
    for path in [&binary, &archive] {
        let unkept = edit(path);
        assert_eq!((unkept.modified, unkept.binary), (None, true), "{path:?}");
    }
    let copy = edit(&latin1).modified.expect("a text file's copy");
    assert!(!edit(&latin1).binary);
    assert_eq!(fs::read(copy).unwrap(), b"caf\xe9\n");
}
