#![cfg(unix)]

use demi_runner::shell::utilities::{Context, run};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    os::fd::OwnedFd,
    sync::Arc,
};

fn context(cwd: &std::path::Path, input: File, output: File, error: File) -> Context {
    Context {
        control: None,
        descriptors: BTreeMap::new(),
        live_input: false,
        umask: 0o022,
        name: "cat",
        cwd: cwd.into(),
        env: BTreeMap::new(),
        stdin: Arc::new(input),
        stdout: Arc::new(output),
        stderr: Arc::new(error),
    }
}

#[test]
fn two_uutils_builtins_stream_through_os_pipes_in_one_process() {
    let root = tempfile::tempdir().unwrap();
    let (input, mut writer) = std::io::pipe().unwrap();
    let (middle_read, middle_write) = std::io::pipe().unwrap();
    let (mut reader, output) = std::io::pipe().unwrap();
    let error = tempfile::tempfile().unwrap();
    let first = context(
        root.path(),
        File::from(OwnedFd::from(input)),
        File::from(OwnedFd::from(middle_write)),
        error.try_clone().unwrap(),
    );
    let second = context(
        root.path(),
        File::from(OwnedFd::from(middle_read)),
        File::from(OwnedFd::from(output)),
        error,
    );
    let first = std::thread::spawn(move || run(first, vec!["cat".into()]).unwrap());
    let second = std::thread::spawn(move || run(second, vec!["cat".into()]).unwrap());
    writer.write_all(&[0, 255, 128, 10]).unwrap();
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes, [0, 255, 128, 10]);
    drop(writer);
    assert_eq!(first.join().unwrap(), 0);
    assert_eq!(second.join().unwrap(), 0);
}

#[test]
fn uutils_cwd_and_exit_state_are_per_invocation() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    std::fs::write(first.path().join("file"), "one").unwrap();
    std::fs::write(second.path().join("file"), "two").unwrap();
    let invoke = |root: &std::path::Path, name: &str| {
        let output = tempfile::NamedTempFile::new().unwrap();
        let error = tempfile::NamedTempFile::new().unwrap();
        let ctx = context(
            root,
            tempfile::tempfile().unwrap(),
            output.reopen().unwrap(),
            error.reopen().unwrap(),
        );
        let code = run(ctx, vec!["cat".into(), name.into()]).unwrap();
        (
            code,
            std::fs::read(output.path()).unwrap(),
            std::fs::read(error.path()).unwrap(),
        )
    };
    assert_eq!(invoke(first.path(), "file").1, b"one");
    assert_eq!(invoke(second.path(), "file").1, b"two");
    let failure = invoke(first.path(), "missing");
    assert_eq!(failure.0, 1);
    assert!(String::from_utf8_lossy(&failure.2).starts_with("cat:"));
    assert_eq!(invoke(second.path(), "file").0, 0);
    assert_eq!(invoke(second.path(), "--help").0, 0);
}
