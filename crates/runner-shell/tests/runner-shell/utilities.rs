use demi_runner_shell::testing::{UTILITIES, run};
use std::{collections::BTreeMap, path::Path, sync::Arc};
use uucore::context::Context;

fn invoke(root: &Path, name: &'static str, args: &[&str]) -> (i32, String, String) {
    let output = tempfile::NamedTempFile::new().unwrap();
    let error = tempfile::NamedTempFile::new().unwrap();
    let context = Context {
        control: None,
        descriptors: BTreeMap::new(),
        live_input: false,
        umask: 0o022,
        name,
        cwd: root.into(),
        env: BTreeMap::new(),
        stdin: Arc::new(tempfile::tempfile().unwrap()),
        stdout: Arc::new(output.reopen().unwrap()),
        stderr: Arc::new(error.reopen().unwrap()),
    };
    let args = std::iter::once(name)
        .chain(args.iter().copied())
        .map(Into::into)
        .collect();
    let code = run(context, args).unwrap();
    (
        code,
        std::fs::read_to_string(output.path()).unwrap(),
        std::fs::read_to_string(error.path()).unwrap(),
    )
}

/// A failed `cat` leaves the next invocation on the same thread its own exit
/// status: uucore keeps the status per thread, and a job's utility threads
/// are reused.
#[test]
fn uutils_cwd_and_exit_state_are_per_invocation() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    std::fs::write(first.path().join("file"), "one").unwrap();
    std::fs::write(second.path().join("file"), "two").unwrap();
    assert_eq!(invoke(first.path(), "cat", &["file"]).1, "one");
    assert_eq!(invoke(second.path(), "cat", &["file"]).1, "two");
    let failure = invoke(first.path(), "cat", &["missing"]);
    assert_eq!(failure.0, 1);
    assert!(failure.2.starts_with("cat:"));
    assert_eq!(invoke(second.path(), "cat", &["file"]).0, 0);
    assert_eq!(invoke(second.path(), "cat", &["--help"]).0, 0);
}

#[test]
fn every_utility_routes_help_to_the_invocation_stream() {
    let root = tempfile::tempdir().unwrap();
    for &(name, _) in UTILITIES {
        let (code, output, error) = invoke(root.path(), name, &["--help"]);
        assert_eq!(code, 0, "{name}: {error}");
        assert!(
            output.to_lowercase().contains("usage:"),
            "{name}: {output:?}; {error}"
        );
    }
}

#[test]
fn filesystem_utilities_use_the_invocation_directory() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("input"), "hello\n").unwrap();
    for (name, args) in [
        ("mkdir", vec!["-p", "nested/child"]),
        ("touch", vec!["new"]),
        ("cp", vec!["input", "copy"]),
        ("mv", vec!["copy", "moved"]),
        ("ls", vec!["-la", "."]),
        ("stat", vec!["input"]),
        ("du", vec!["input"]),
        ("df", vec!["."]),
        ("rm", vec!["moved"]),
        ("rmdir", vec!["nested/child"]),
    ] {
        let (code, output, error) = invoke(root.path(), name, &args);
        assert_eq!(code, 0, "{name}: {output}; {error}");
    }
    assert!(root.path().join("new").is_file());
    assert!(!root.path().join("moved").exists());
}

#[cfg(unix)]
#[test]
fn env_executes_child_with_local_environment_and_streams() {
    let root = tempfile::tempdir().unwrap();
    let (code, output, error) = invoke(
        root.path(),
        "env",
        &[
            "-i",
            "DEMI_TEST_VALUE=local",
            "/bin/sh",
            "-c",
            "printf '%s' \"$DEMI_TEST_VALUE\"; pwd; exit 7",
        ],
    );
    assert_eq!(code, 7, "{error}");
    assert_eq!(
        output,
        format!("local{}\n", root.path().canonicalize().unwrap().display())
    );
    assert!(std::env::var_os("DEMI_TEST_VALUE").is_none());
}

#[test]
fn recursive_operations_stay_in_the_invocation_directory() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("source/child")).unwrap();
    std::fs::write(root.path().join("source/child/file"), "nested").unwrap();
    for (name, args) in [
        ("cp", vec!["-R", "source", "copy"]),
        ("du", vec!["copy"]),
        ("ls", vec!["-R", "copy"]),
        ("rm", vec!["-r", "copy"]),
    ] {
        let (code, output, error) = invoke(root.path(), name, &args);
        assert_eq!(code, 0, "{name}: {output}; {error}");
        if name == "cp" {
            assert_eq!(
                std::fs::read_to_string(root.path().join("copy/child/file")).unwrap(),
                "nested"
            );
        }
    }
    assert!(!root.path().join("copy").exists());
    assert!(root.path().join("source/child/file").exists());
}

#[test]
fn concurrent_external_sorts_own_their_temporary_files_and_workers() {
    let jobs: Vec<_> = (0..2)
        .map(|index| {
            std::thread::spawn(move || {
                let root = tempfile::tempdir().unwrap();
                std::fs::create_dir(root.path().join("temporary")).unwrap();
                let text: String = (0..2000)
                    .rev()
                    .map(|number| format!("{number:04}-{index}\n"))
                    .collect();
                std::fs::write(root.path().join("input"), text).unwrap();
                let (code, output, error) = invoke(
                    root.path(),
                    "sort",
                    &["--parallel=2", "-S", "1K", "-T", "temporary", "input"],
                );
                assert_eq!(code, 0, "{error}");
                let expected: String = (0..2000)
                    .map(|number| format!("{number:04}-{index}\n"))
                    .collect();
                assert_eq!(output, expected);
                assert_eq!(
                    std::fs::read_dir(root.path().join("temporary"))
                        .unwrap()
                        .count(),
                    0
                );
            })
        })
        .collect();
    for job in jobs {
        job.join().unwrap();
    }
}

#[cfg(unix)]
#[test]
fn relative_symlinks_and_explicit_directory_modes_are_preserved() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("target"), "bytes").unwrap();
    symlink("target", root.path().join("link")).unwrap();
    for (name, args) in [
        ("cp", vec!["-P", "link", "copy"]),
        ("touch", vec!["-h", "link"]),
        ("mkdir", vec!["-m", "0777", "directory"]),
    ] {
        let (code, output, error) = invoke(root.path(), name, &args);
        assert_eq!(code, 0, "{name}: {output}; {error}");
    }
    assert_eq!(
        std::fs::read_link(root.path().join("copy")).unwrap(),
        Path::new("target")
    );
    assert_eq!(
        std::fs::metadata(root.path().join("directory"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o777
    );
}

#[test]
fn search_edit_and_compare_utilities_keep_their_cli_and_local_paths() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("tree")).unwrap();
    std::fs::write(root.path().join("tree/input"), "apple\npear\n").unwrap();
    let (code, output, error) = invoke(root.path(), "grep", &["-rn", "apple", "tree"]);
    assert_eq!(code, 0, "{error}");
    let input = Path::new("tree").join("input");
    assert_eq!(output, format!("{}:1:apple\n", input.display()));
    let (code, output, error) = invoke(
        root.path(),
        "find",
        &["tree", "-type", "f", "-name", "input"],
    );
    assert_eq!(code, 0, "{error}");
    assert_eq!(output, format!("{}\n", input.display()));
    let (code, output, error) =
        invoke(root.path(), "sed", &["-i", "s/apple/orange/", "tree/input"]);
    assert_eq!(code, 0, "{output}; {error}");
    assert_eq!(
        std::fs::read_to_string(root.path().join("tree/input")).unwrap(),
        "orange\npear\n"
    );
    std::fs::write(root.path().join("other"), "different\n").unwrap();
    for name in ["diff", "cmp"] {
        let (code, _, error) = invoke(root.path(), name, &["tree/input", "other"]);
        assert_eq!(code, 1, "{name}: {error}");
        let (code, _, error) = invoke(root.path(), name, &["tree/input", "tree/input"]);
        assert_eq!(code, 0, "{name}: {error}");
    }
}

/// `sed -i` with GNU sed's attached suffix, alone or after other short
/// options; the script after a bare `-i` and the token after `-e` are never
/// a suffix. BSD's separate suffix is covered with edit tracking.
#[test]
fn sed_in_place_takes_an_attached_suffix_as_gnu_sed_does() {
    let root = tempfile::tempdir().unwrap();
    let cases: [(&[&str], Option<&str>); 6] = [
        (&["-i", "s/a/b/", "f"], None),
        (&["-i.bak", "s/a/b/", "f"], Some("f.bak")),
        (&["-ni.orig", "s/a/b/p", "f"], Some("f.orig")),
        (&["--in-place=.old", "s/a/b/", "f"], Some("f.old")),
        (&["-e", "s/a/b/", "-i~", "f"], Some("f~")),
        (&["-E", "-i", "-e", "s/(a)/b/", "f"], None),
    ];
    for (args, backup) in cases {
        std::fs::write(root.path().join("f"), "a\n").unwrap();
        let (code, output, error) = invoke(root.path(), "sed", args);
        assert_eq!(code, 0, "{args:?}: {output}; {error}");
        assert_eq!(std::fs::read_to_string(root.path().join("f")).unwrap(), "b\n", "{args:?}");
        if let Some(backup) = backup {
            let path = root.path().join(backup);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\n", "{args:?}");
            std::fs::remove_file(path).unwrap();
        }
        let left: Vec<_> = std::fs::read_dir(root.path()).unwrap().collect();
        assert_eq!(left.len(), 1, "{args:?} left another file");
    }
}

#[test]
fn jq_uses_full_filters_files_arguments_and_invocation_environment() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("input.json"), r#"[{"x":1},{"x":3}]"#).unwrap();
    let (code, output, error) = invoke(
        root.path(),
        "jq",
        &[
            "-c",
            "--argjson",
            "min",
            "2",
            "map(select(.x > $min)) | .[].x",
            "input.json",
        ],
    );
    assert_eq!(code, 0, "{error}");
    assert_eq!(output, "3\n");
    let (code, output, error) = invoke(
        root.path(),
        "jq",
        &["-n", "-c", "env == $ENV and (env | length == 0)"],
    );
    assert_eq!(code, 0, "{error}");
    assert_eq!(output, "true\n");
    let (code, _, _) = invoke(root.path(), "jq", &["-n", "-e", "false"]);
    assert_eq!(code, 1);
    let (code, _, _) = invoke(root.path(), "jq", &["-n", "bad_syntax("]);
    assert_eq!(code, 3);
}

#[test]
fn ripgrep_searches_and_filters_local_files_with_upstream_options() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("tree")).unwrap();
    std::fs::write(root.path().join("tree/.gitignore"), "ignored\n").unwrap();
    std::fs::write(root.path().join("tree/ignored"), "apple\n").unwrap();
    std::fs::write(root.path().join("tree/input"), "apple\npear\nAPPLE\n").unwrap();
    let (code, output, error) = invoke(
        root.path(),
        "rg",
        &["-j2", "--no-require-git", "-ni", "apple", "tree"],
    );
    assert_eq!(code, 0, "{error}");
    let input = Path::new("tree").join("input");
    assert_eq!(
        output,
        format!("{0}:1:apple\n{0}:3:APPLE\n", input.display())
    );
    let (code, output, error) = invoke(root.path(), "rg", &["--json", "pear", "tree/input"]);
    assert_eq!(code, 0, "{error}");
    assert!(output.contains("\"type\":\"match\""));
    let (code, _, _) = invoke(root.path(), "rg", &["not-present", "tree"]);
    assert_eq!(code, 1);
    let (code, _, _) = invoke(root.path(), "rg", &["[", "tree"]);
    assert_eq!(code, 2);
}

/// A fixed bug: a cancelled utility unwinds, and a buffered writer it held
/// flushes on the way out; that write found the invocation cancelled and
/// unwound again inside the destructor, which aborted the whole runner. The
/// flush now fails, and only the invocation ends.
#[test]
fn a_cancelled_utility_unwinds_through_a_buffered_writer_without_aborting() {
    struct Cancelled;
    impl uucore::context::Control for Cancelled {
        fn check(&self) -> std::io::Result<()> {
            Err(std::io::ErrorKind::Interrupted.into())
        }
        fn read(&self, _: &std::fs::File, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("cancelled"))
        }
        fn write(&self, _: &std::fs::File, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("cancelled"))
        }
        fn sleep(&self, _: std::time::Duration) -> std::io::Result<()> {
            Err(std::io::ErrorKind::Interrupted.into())
        }
        fn resolve(&self, path: &Path, cwd: &Path) -> std::path::PathBuf {
            cwd.join(path)
        }
        fn task_guard(&self) -> Box<dyn Send + Sync> {
            Box::new(())
        }
    }
    let root = tempfile::tempdir().unwrap();
    let context = Context {
        control: Some(Arc::new(Cancelled)),
        descriptors: BTreeMap::new(),
        live_input: false,
        umask: 0o022,
        name: "cat",
        cwd: root.path().into(),
        env: BTreeMap::new(),
        stdin: Arc::new(tempfile::tempfile().unwrap()),
        stdout: Arc::new(tempfile::tempfile().unwrap()),
        stderr: Arc::new(tempfile::tempfile().unwrap()),
    };
    let unwound = uucore::context::with(context, || {
        std::panic::catch_unwind(|| {
            use std::io::Write;
            let mut output = std::io::BufWriter::new(uucore::context::io::stdout());
            output.write_all(b"held until the writer drops").unwrap();
            uucore::context::check_cancelled();
        })
    });
    assert!(unwound.unwrap_err().is::<uucore::context::Cancelled>());
}
