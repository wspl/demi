#![cfg(unix)]
//! With no open file left, recording an edit waits for one rather than leave
//! the edit out (`runner.md` § Load). Its own binary: it lowers the process's
//! open-file limit and holds every remaining descriptor.

use std::{fs, path::Path, time::Duration};

use demi_command_service::{descriptors, edits::Recorder, protocol::EditContext};

/// Every descriptor the process has left, held open.
struct Hog(Vec<fs::File>);

impl Hog {
    fn fill() -> Self {
        let mut files = Vec::new();
        loop {
            match fs::File::open("/dev/null") {
                Ok(file) => files.push(file),
                Err(error) if error.raw_os_error() == Some(libc::EMFILE) => return Self(files),
                Err(error) => panic!("filling descriptors: {error}"),
            }
        }
    }
}

fn set_soft_limit(value: u64) {
    // SAFETY: getrlimit and setrlimit only read and write the given struct.
    unsafe {
        let mut limit = std::mem::zeroed::<libc::rlimit>();
        assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit), 0);
        limit.rlim_cur = value as libc::rlim_t;
        assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &limit), 0);
    }
}

fn recorder(root: &Path) -> Recorder {
    Recorder::new(EditContext {
        directory: root.join("job").to_string_lossy().into_owned(),
        lock: root.join("edits.lock").to_string_lossy().into_owned(),
    })
    .unwrap()
}

#[test]
fn recording_an_edit_waits_for_an_open_file() {
    set_soft_limit(512);
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file");
    fs::write(&path, "before").unwrap();
    let recorder = recorder(root.path());
    let mut hog = Hog::fill();
    let writer = std::thread::spawn({
        let path = path.clone();
        move || {
            // The write the recorder observes waits the same way the
            // runner's own file operations do.
            let written = recorder.record(&path, || {
                descriptors::retry_blocking(|| fs::write(&path, "after"))
            });
            (recorder, written)
        }
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !writer.is_finished(),
        "recording did not wait with no open file left"
    );
    let keep = hog.0.len().saturating_sub(64);
    hog.0.truncate(keep);
    let (recorder, written) = writer.join().unwrap();
    written.unwrap();
    drop(hog);
    let report = recorder.report().unwrap();
    assert_eq!(report.files.len(), 1, "the edit was left out: {report:?}");
    let edit = &report.files[0].edits[0];
    let original = fs::read_to_string(edit.original.as_ref().unwrap()).unwrap();
    let modified = fs::read_to_string(edit.modified.as_ref().unwrap()).unwrap();
    assert_eq!((original.as_str(), modified.as_str()), ("before", "after"));
}
