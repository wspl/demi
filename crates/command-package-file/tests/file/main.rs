//! The `demi.file` program at its boundary (`commands.md` § File commands):
//! the command service a runner starts, its file operations, and the edits
//! it records. As the package's integration tests, they also make `cargo
//! test` build the program, which the runner's and the backend's tests start.

use std::{collections::BTreeMap, path::Path, time::Duration};

use demi_command_protocol::{
    CommandCaller, CommandContext, CommandLocale, Completion, EditContext, Invocation, Record,
};
use demi_command_sdk::{Client, testing::ServiceProcess};

/// Where a test's invocations record their edits: beside the files, under
/// `cwd`.
fn edits(cwd: &str) -> EditContext {
    EditContext {
        directory: Path::new(cwd)
            .join("changes")
            .to_string_lossy()
            .into_owned(),
        lock: Path::new(cwd)
            .join("edits.lock")
            .to_string_lossy()
            .into_owned(),
    }
}

/// Runs `operation` with `args` in `cwd`: its completion, standard output
/// and standard error.
async fn call(
    client: &Client,
    cwd: &str,
    operation: &str,
    args: serde_json::Value,
) -> (Completion, Vec<u8>, Vec<u8>) {
    let request = Invocation {
        context: CommandContext {
            color_scheme: demi_command_protocol::ColorScheme::Light,
            conversation: "file-test-conversation".into(),
            caller: CommandCaller::agent(1),
            locale: CommandLocale {
                time_zone: "UTC".into(),
                languages: vec!["en-US".into()],
            },
        },
        json: None,
        edits: Some(edits(cwd)),
        operation: operation.into(),
        invocation_id: operation.into(),
        args,
        cwd: cwd.into(),
        env: BTreeMap::new(),
        stdout: None,
    };
    // The file commands never ask for input: the request stays open without
    // input or its end.
    let (_input, mut output) = client.invoke(&request).await.unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut completion = None;
    while let Some(record) = output.next().await.unwrap() {
        match record {
            Record::Stdout(bytes) => stdout.extend_from_slice(&bytes),
            Record::Stderr(bytes) => stderr.extend_from_slice(&bytes),
            Record::Completion(value) => completion = Some(value),
            Record::InputPull => panic!("a file operation reads no raw input"),
            Record::Medium { .. } | Record::MediumBytes(_) => {
                panic!("an invocation that is no job command's returns no media")
            }
        }
    }
    (completion.unwrap(), stdout, stderr)
}

#[tokio::test]
async fn the_resident_program_serves_every_file_operation_and_records_its_edits() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().to_str().unwrap();
        let service = ServiceProcess::start(env!("CARGO_BIN_EXE_demi-file"), &["--command-service"], &[])
            .await
            .unwrap();
        let pid = service.id().unwrap();
        let client = service.client();
        assert!(client.info().await.unwrap().operations.contains(&"file.read".into()));
        let (result, output, _) = call(client, cwd, "file.create", serde_json::json!({"path":"nested/a.txt", "content":"alpha\nbeta\n"})).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(output, b"Created nested/a.txt\n");
        let (result, _, _) = call(client, cwd, "file.create", serde_json::json!({"path":"nested/a.txt", "content":"overwrite"})).await;
        assert_eq!(result.exit_code, 1);
        let (result, _, _) = call(client, cwd, "file.edit", serde_json::json!({"path":"nested/a.txt", "old":"beta", "new":"gamma"})).await;
        assert_eq!(result.exit_code, 0);
        let patch = "--- a/nested/a.txt\n+++ b/nested/a.txt\n@@ -1,2 +1,2 @@\n alpha\n-gamma\n+delta\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+created\n";
        let (result, output, error) = call(client, cwd, "file.patch", serde_json::json!({"patch":patch})).await;
        assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&error));
        assert_eq!(output, b"Patched 2 file(s)\n");
        let (_, output, _) = call(client, cwd, "file.read", serde_json::json!({"path":"nested/a.txt"})).await;
        assert_eq!(output, b"alpha\ndelta\n");
        let binary = [0, 255, 10, 13, 128];
        std::fs::write(root.path().join("image.bin"), binary).unwrap();
        let (result, output, _) = call(client, cwd, "file.read", serde_json::json!({"path":"image.bin"})).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(output, binary);
        let recorder = demi_command_sdk::edits::Recorder::new(demi_command_protocol::EditContext {
            directory: root.path().join("changes").to_string_lossy().into_owned(),
            lock: root.path().join("edits.lock").to_string_lossy().into_owned(),
        }).unwrap();
        let report = recorder.report().unwrap();
        let large = root.path().join("large.txt");
        std::fs::write(&large, "x".repeat(demi_command_protocol::EDIT_FILE_BYTES + 1)).unwrap();
        for (operation, args) in [
            ("file.create", serde_json::json!({"path":"large.txt", "content":"overwrite"})),
            ("file.edit", serde_json::json!({"path":"large.txt", "old":"absent", "new":"replacement"})),
            ("file.patch", serde_json::json!({"patch":"--- a/large.txt\n+++ b/large.txt\n@@ -1 +1 @@\n-absent\n+replacement\n"})),
        ] {
            let (result, _, _) = call(client, cwd, operation, args).await;
            assert_eq!(result.exit_code, 1);
        }
        assert_eq!(recorder.report().unwrap().files.len(), 2);
        assert_eq!(report.files.len(), 2);
        assert_eq!(
            report.files[0].kind,
            demi_command_protocol::EditKind::Added
        );
        assert_eq!(report.files[0].edits.len(), 1);
        assert_eq!(std::fs::read(report.files[0].edits[0].modified.as_ref().unwrap()).unwrap(), b"alpha\ndelta\n");
        assert_eq!(std::fs::read(report.files[1].edits[0].modified.as_ref().unwrap()).unwrap(), b"created\n");
        assert_eq!(service.id(), Some(pid));
        let status = service.shutdown().await.unwrap();
        assert!(status.success());
    }).await.unwrap();
}

/// A patch whose later file cannot be written leaves every file as it was:
/// the earlier one it wrote is restored, and no edit is recorded.
#[cfg(unix)]
#[tokio::test]
async fn a_patch_that_fails_at_a_later_file_restores_the_earlier_ones() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().to_str().unwrap();
        std::fs::write(root.path().join("first.txt"), "first\n").unwrap();
        let locked = Locked::new(root.path());
        let service =
            ServiceProcess::start(env!("CARGO_BIN_EXE_demi-file"), &["--command-service"], &[])
                .await
                .unwrap();
        let second = format!("locked/{}", locked.name);
        let text = locked.text.trim_end();
        let patch = format!(
            "--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n\
             --- a/{second}\n+++ b/{second}\n@@ -1 +1 @@\n-{text}\n+changed\n"
        );
        let (result, _, error) = call(
            service.client(),
            cwd,
            "file.patch",
            serde_json::json!({"patch": patch}),
        )
        .await;
        let left = locked.read();
        assert_eq!(result.exit_code, 1, "{}", String::from_utf8_lossy(&error));
        assert_eq!(
            std::fs::read_to_string(root.path().join("first.txt")).unwrap(),
            "first\n"
        );
        assert_eq!(left, locked.text);
        let recorder = demi_command_sdk::edits::Recorder::new(edits(cwd)).unwrap();
        assert!(recorder.report().unwrap().files.is_empty());
        assert!(service.shutdown().await.unwrap().success());
    })
    .await
    .unwrap();
}

/// `locked` under a test's directory: a directory in which no file can be
/// published, and the one-line file it holds. On Linux it links to
/// `/proc/sys/kernel`, where nobody makes a file, root included, so the
/// test does not depend on who runs it; elsewhere it is a directory without
/// write permission, which binds everyone but root.
#[cfg(unix)]
struct Locked {
    directory: std::path::PathBuf,
    name: &'static str,
    text: String,
}

#[cfg(unix)]
impl Locked {
    #[cfg(target_os = "linux")]
    fn new(root: &std::path::Path) -> Self {
        let directory = root.join("locked");
        std::os::unix::fs::symlink("/proc/sys/kernel", &directory).unwrap();
        let name = "ostype";
        let text = std::fs::read_to_string(directory.join(name)).unwrap();
        Self {
            directory,
            name,
            text,
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn new(root: &std::path::Path) -> Self {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = root.join("locked");
        std::fs::create_dir(&directory).unwrap();
        let (name, text) = ("second.txt", "second\n".to_owned());
        std::fs::write(directory.join(name), &text).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o555)).unwrap();
        Self {
            directory,
            name,
            text,
        }
    }

    fn read(&self) -> String {
        std::fs::read_to_string(self.directory.join(self.name)).unwrap()
    }
}

/// The directory takes its permissions back, so the test's directory can
/// be removed; a failure only leaves that temporary directory behind.
#[cfg(all(unix, not(target_os = "linux")))]
impl Drop for Locked {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt as _;

        let _ = std::fs::set_permissions(&self.directory, std::fs::Permissions::from_mode(0o755));
    }
}
