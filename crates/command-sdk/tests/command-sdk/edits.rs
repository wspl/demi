//! The invocation edit recorder (`edit-tracking.md`): what it records of a
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

#[test]
fn binary_edits_have_no_contents() {
    let root = tempfile::tempdir().unwrap();
    let recorder = recorder(root.path(), "job");
    let path = root.path().join("binary");
    recorder
        .record(&path, || fs::write(&path, [0, 1, 2]))
        .unwrap();
    let report = recorder.report().unwrap();
    assert_eq!(report.files[0].kind, EditKind::Added);
    assert!(report.files[0].edits[0].modified.is_none());
}
