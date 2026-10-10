//! The `demi.file` program at its boundary (`commands.md` § File commands):
//! the command service a runner starts, its file operations, and the edits
//! it records. As the package's integration tests, they also make `cargo
//! test` build the program, which the runner's and the backend's tests start.

use std::{collections::BTreeMap, path::Path, time::Duration};

use bytes::Bytes;
use demi_command_protocol::{
    CommandCaller, CommandContext, CommandLocale, Completion, EditContext, Invocation,
    MediumFacts, Record, Viewable,
};
use demi_command_sdk::{
    Client, Exchange, InputSource, OutputSink, ServiceError, testing::ServiceProcess,
};

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
        command: "demi test".into(),
        args,
        cwd: cwd.into(),
        env: BTreeMap::new(),
        live_input: None,
        viewable: None,
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
        assert!(client.info().await.unwrap().operations.contains(&"file.view".into()));
        let create = |content: &str| serde_json::json!({"blocks": format!("nested/a.txt\n<<<<<<< SEARCH\n=======\n{content}>>>>>>> REPLACE\n")});
        let (result, output, _) = call(client, cwd, "file.edit", create("alpha\nbeta\n")).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(output, b"Created nested/a.txt (2 lines)\n");
        let (result, _, _) = call(client, cwd, "file.edit", create("overwrite\n")).await;
        assert_eq!(result.exit_code, 1);
        let (result, _, _) = call(client, cwd, "file.edit", serde_json::json!({"path":"nested/a.txt", "old":"beta", "new":"gamma"})).await;
        assert_eq!(result.exit_code, 0);
        let patch = "--- a/nested/a.txt\n+++ b/nested/a.txt\n@@ -1,2 +1,2 @@\n alpha\n-gamma\n+delta\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+created\n";
        let (result, output, error) = call(client, cwd, "file.patch", serde_json::json!({"patch":patch})).await;
        assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&error));
        assert_eq!(output, b"Patched 2 file(s)\n");
        assert_eq!(std::fs::read(root.path().join("nested/a.txt")).unwrap(), b"alpha\ndelta\n");
        let recorder = demi_command_sdk::edits::Recorder::new(demi_command_protocol::EditContext {
            directory: root.path().join("changes").to_string_lossy().into_owned(),
            lock: root.path().join("edits.lock").to_string_lossy().into_owned(),
        }).unwrap();
        let report = recorder.report().unwrap();
        let large = root.path().join("large.txt");
        std::fs::write(&large, "x".repeat(demi_command_protocol::EDIT_FILE_BYTES + 1)).unwrap();
        for (operation, args) in [
            ("file.edit", serde_json::json!({"path":"large.txt", "blocks":"<<<<<<< SEARCH\n=======\noverwrite\n>>>>>>> REPLACE\n"})),
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

/// Starts the program for one test, in a directory holding `files`.
async fn service_with(files: &[(&str, &str)]) -> (tempfile::TempDir, ServiceProcess) {
    let root = tempfile::tempdir().unwrap();
    for (name, content) in files {
        std::fs::write(root.path().join(name), content).unwrap();
    }
    let service = ServiceProcess::start(env!("CARGO_BIN_EXE_demi-file"), &["--command-service"], &[])
        .await
        .unwrap();
    (root, service)
}

/// What a failed call wrote: its completion's message.
fn message(completion: &Completion) -> &str {
    &completion.error.as_ref().unwrap().message
}

/// A block's SEARCH is text found exactly once anywhere in the file, part
/// of a line included; every block applies together or none does, in the
/// file's own line endings, and an error names the file by its full path.
#[tokio::test]
async fn search_replace_blocks_replace_text_found_once_all_together() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let (root, service) = service_with(&[
            ("twice.txt", "start\nrepeat\nmiddle\nrepeat\nend\n"),
            ("two.txt", "alpha\nbeta\ngamma\ndelta\n"),
            ("crlf.txt", "one\r\ntwo\r\nthree\r\n"),
            ("call.ts", "const limit = withinLimit(5000);\nexport { withinLimit };\n"),
            ("runs.txt", "x\nx\nx\n"),
        ])
        .await;
        let cwd = root.path().to_str().unwrap();
        // The parent directory as the system resolves it: on macOS a
        // temporary directory under /var is /private/var.
        let full = |name: &str| std::fs::canonicalize(root.path()).unwrap().join(name).display().to_string();
        let read = |name: &str| std::fs::read_to_string(root.path().join(name)).unwrap();
        let edit = |path: &str, blocks: &str| {
            call(
                service.client(),
                cwd,
                "file.edit",
                serde_json::json!({"path": path, "blocks": blocks}),
            )
        };

        // A SEARCH that occurs twice names the line of each, and changes
        // nothing.
        let (result, _, _) = edit("twice.txt", "<<<<<<< SEARCH\nrepeat\n=======\nonce\n>>>>>>> REPLACE\n").await;
        assert_eq!(result.exit_code, 1);
        assert_eq!(
            message(&result),
            format!("{}: block 1: its SEARCH occurs 2 times, at lines 2 and 4; include more of the text around it so it occurs once, and nothing was written", full("twice.txt"))
        );
        assert_eq!(read("twice.txt"), "start\nrepeat\nmiddle\nrepeat\nend\n");

        // A second block that is not in the file names itself and the
        // closest lines, and the first block's change is not made either.
        let failing = "<<<<<<< SEARCH\nalpha\n=======\nALPHA\n>>>>>>> REPLACE\n\n<<<<<<< SEARCH\ngamma\ndelto\n=======\n>>>>>>> REPLACE\n";
        let (result, _, _) = edit("two.txt", failing).await;
        assert_eq!(result.exit_code, 1);
        assert_eq!(
            message(&result),
            format!("{}: block 2: its SEARCH is not in the file; a SEARCH must match the file's text exactly, whitespace included, and nothing was written. The closest lines are 3-4:\n3: gamma\n4: delta", full("two.txt"))
        );
        assert_eq!(read("two.txt"), "alpha\nbeta\ngamma\ndelta\n");

        // Two blocks apply together, each against the file as it was; an
        // empty REPLACE deletes its lines with their line endings, and a
        // marker may carry trailing spaces.
        let both = "<<<<<<< SEARCH  \nalpha\n=======\nALPHA\n>>>>>>> REPLACE\n<<<<<<< SEARCH\ngamma\ndelta\n======= \n>>>>>>> REPLACE \n";
        let (result, output, _) = edit("two.txt", both).await;
        assert_eq!(result.exit_code, 0, "{result:?}");
        assert_eq!(String::from_utf8(output).unwrap(), "Edited two.txt (+1 \u{2212}3)\n   1  ALPHA\n   2  beta\n");
        assert_eq!(read("two.txt"), "ALPHA\nbeta\n");

        // Part of a line changes only that part.
        let (result, _, _) = edit("call.ts", "<<<<<<< SEARCH\nwithinLimit(\n=======\nboundedWait(\n>>>>>>> REPLACE\n").await;
        assert_eq!(result.exit_code, 0, "{result:?}");
        assert_eq!(read("call.ts"), "const limit = boundedWait(5000);\nexport { withinLimit };\n");

        // Overlapping occurrences count as one: the first is replaced.
        let (result, _, _) = edit("runs.txt", "<<<<<<< SEARCH\nx\nx\n=======\ny\n>>>>>>> REPLACE\n").await;
        assert_eq!(result.exit_code, 0, "{result:?}");
        assert_eq!(read("runs.txt"), "y\nx\n");

        // A file with CRLF line endings matches a SEARCH across its lines
        // and keeps its endings, though the blocks come with LF.
        let (result, _, _) = edit("crlf.txt", "<<<<<<< SEARCH\ntwo\nthree\n=======\n2a\n2b\n>>>>>>> REPLACE\n").await;
        assert_eq!(result.exit_code, 0, "{result:?}");
        assert_eq!(read("crlf.txt"), "one\r\n2a\r\n2b\r\n");

        // A file that does not exist is named by its full path, as the
        // system words it: its parent resolved when it exists, `..`
        // included, or else the path as joined to the working directory.
        std::fs::create_dir(root.path().join("sub")).unwrap();
        let (result, _, _) = edit("sub/../absent.txt", "<<<<<<< SEARCH\nx\n=======\ny\n>>>>>>> REPLACE\n").await;
        assert_eq!(result.exit_code, 1);
        assert_eq!(message(&result), format!("{}: No such file or directory", full("absent.txt")));
        let (result, _, _) = edit("nodir/absent.txt", "<<<<<<< SEARCH\nx\n=======\ny\n>>>>>>> REPLACE\n").await;
        assert_eq!(
            message(&result),
            format!("{}: No such file or directory", root.path().join("nodir/absent.txt").display())
        );
        assert!(service.shutdown().await.unwrap().success());
    })
    .await
    .unwrap();
}

/// What deletes a line ending and what does not: only a SEARCH of whole
/// lines with no REPLACE lines at all takes its last one; a REPLACE of one
/// blank line leaves one, a SEARCH of blank lines alone is refused, and
/// `--old` fails as a block does.
#[tokio::test]
async fn deletions_keep_lines_apart_and_old_fails_as_blocks_do() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let (root, service) = service_with(&[
            ("note.js", "x(); // note\nnext\n"),
            ("blank.txt", "a\nb\nc\n"),
            ("gaps.txt", "a\n\n\nb\n\n\nc\n"),
            ("old.txt", "alpha\nbeta\nbeta\n"),
        ])
        .await;
        let cwd = root.path().to_str().unwrap();
        let full = |name: &str| std::fs::canonicalize(root.path()).unwrap().join(name).display().to_string();
        let read = |name: &str| std::fs::read_to_string(root.path().join(name)).unwrap();
        let edit = |args: serde_json::Value| call(service.client(), cwd, "file.edit", args);

        // The end of a line, deleted, leaves the line and the next apart.
        let (result, _, _) = edit(serde_json::json!({"path": "note.js", "blocks": "<<<<<<< SEARCH\n // note\n=======\n>>>>>>> REPLACE\n"})).await;
        assert_eq!(result.exit_code, 0, "{result:?}");
        assert_eq!(read("note.js"), "x();\nnext\n");

        // A REPLACE of one blank line leaves a blank line.
        let (result, _, _) = edit(serde_json::json!({"path": "blank.txt", "blocks": "<<<<<<< SEARCH\nb\n=======\n\n>>>>>>> REPLACE\n"})).await;
        assert_eq!(result.exit_code, 0, "{result:?}");
        assert_eq!(read("blank.txt"), "a\n\nc\n");

        // Blank lines alone name no one place.
        let (result, _, _) = edit(serde_json::json!({"path": "gaps.txt", "blocks": "<<<<<<< SEARCH\n\n\n=======\n>>>>>>> REPLACE\n"})).await;
        assert_eq!(
            message(&result),
            format!("{}: block 1: its SEARCH holds only blank lines, which match too many places; include a line of text around it, and nothing was written", full("gaps.txt"))
        );

        let (result, _, _) = edit(serde_json::json!({"path": "old.txt", "old": "betta", "new": "x"})).await;
        assert_eq!(
            message(&result),
            format!("{}: --old is not in the file; it must match the file's text exactly, whitespace included, and nothing was written. The closest line is 2:\n2: beta", full("old.txt"))
        );
        let (result, _, _) = edit(serde_json::json!({"path": "old.txt", "old": "beta", "new": "x"})).await;
        assert_eq!(
            message(&result),
            format!("{}: --old occurs 2 times, at lines 2 and 3; choose one with --occurrence or --context, or include more of the text, and nothing was written", full("old.txt"))
        );
        assert_eq!(read("old.txt"), "alpha\nbeta\nbeta\n");
        assert!(service.shutdown().await.unwrap().success());
    })
    .await
    .unwrap();
}

/// One call creates a file, edits another and replaces a section of a
/// third, all together: a block that fails in the last file leaves the
/// first two as they were, and the result shows each change as the file
/// now reads.
#[tokio::test]
async fn one_edit_changes_several_files_together_or_none() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let (root, service) = service_with(&[
            ("b.txt", "one\n"),
            ("c.rs", "fn other() {\n    send();\n}\n\nfn serve() {\n    let a = 1;\n    let b = 2;\n    send();\n}\n"),
        ])
        .await;
        let cwd = root.path().to_str().unwrap();
        let read = |name: &str| std::fs::read_to_string(root.path().join(name)).ok();
        let edit = |blocks: String| {
            call(service.client(), cwd, "file.edit", serde_json::json!({"blocks": blocks}))
        };
        let three = |serve_last_line: &str| {
            format!(
                "a.txt\n<<<<<<< SEARCH\n=======\nnew file\n>>>>>>> REPLACE\n\n\
                 b.txt\n<<<<<<< SEARCH\none\n=======\none\ntwo\n>>>>>>> REPLACE\n\n\
                 c.rs\n<<<<<<< SEARCH\nfn serve() {{\n.......\n    {serve_last_line}\n}}\n=======\n\
                 fn serve() {{\n.......\n    stream::send();\n}}\n>>>>>>> REPLACE\n"
            )
        };

        // The third file's block matches nowhere: no file changes, and the
        // message names the file and its block.
        let (result, _, _) = edit(three("absent();")).await;
        assert_eq!(result.exit_code, 1);
        assert!(message(&result).starts_with(&format!("{}: block 1: its SEARCH is not in the file;", std::fs::canonicalize(root.path()).unwrap().join("c.rs").display())), "{}", message(&result));
        assert_eq!(read("a.txt"), None);
        assert_eq!(read("b.txt").unwrap(), "one\n");

        // Applied together: the section's body is kept.
        let (result, output, _) = edit(three("send();")).await;
        assert_eq!(result.exit_code, 0, "{result:?}");
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "Created a.txt (1 line)\n\
             Edited b.txt (+1)\n   1  one\n   2  two\n\
             Edited c.rs (+1 \u{2212}1)\n   7      let b = 2;\n   8      stream::send();\n   9  }\n"
        );
        assert_eq!(read("a.txt").unwrap(), "new file\n");
        assert_eq!(read("b.txt").unwrap(), "one\ntwo\n");
        assert_eq!(
            read("c.rs").unwrap(),
            "fn other() {\n    send();\n}\n\nfn serve() {\n    let a = 1;\n    let b = 2;\n    stream::send();\n}\n"
        );

        // An empty SEARCH never overwrites a file that exists.
        let (result, _, _) = edit("b.txt\n<<<<<<< SEARCH\n=======\nreplaced\n>>>>>>> REPLACE\n".into()).await;
        assert_eq!(result.exit_code, 1);
        assert!(message(&result).starts_with(&format!("{}: block 1: the file exists", std::fs::canonicalize(root.path()).unwrap().join("b.txt").display())), "{}", message(&result));
        assert_eq!(read("b.txt").unwrap(), "one\ntwo\n");
        assert!(service.shutdown().await.unwrap().success());
    })
    .await
    .unwrap();
}

/// A section's REPLACE keeps its lines only with as many seven-dot lines as
/// its SEARCH; a section that matches two places names both.
#[tokio::test]
async fn a_section_keeps_its_lines_and_matches_one_place() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let twice = "fn a() {\n    x();\n}\nfn a() {\n    y();\n}\n";
        let (root, service) = service_with(&[("twice.rs", twice)]).await;
        let cwd = root.path().to_str().unwrap();
        let edit = |blocks: &str| {
            call(service.client(), cwd, "file.edit", serde_json::json!({"path": "twice.rs", "blocks": blocks}))
        };
        let (result, _, _) = edit("<<<<<<< SEARCH\nfn a() {\n.......\n}\n=======\nfn b() {\n.......\n.......\n}\n>>>>>>> REPLACE\n").await;
        assert_eq!(result.exit_code, 1);
        assert_eq!(
            message(&result),
            format!(
                "{}: block 1: its REPLACE has 2 ....... line(s); it needs none, to replace the whole match, or 1, one for each in its SEARCH",
                std::fs::canonicalize(root.path()).unwrap().join("twice.rs").display()
            )
        );
        let (result, _, _) = edit("<<<<<<< SEARCH\nfn a() {\n.......\n}\n=======\nfn b() {\n.......\n}\n>>>>>>> REPLACE\n").await;
        assert_eq!(result.exit_code, 1);
        assert!(message(&result).contains("block 1: its SEARCH occurs 2 times, at lines 1 and 4;"), "{}", message(&result));
        assert_eq!(std::fs::read_to_string(root.path().join("twice.rs")).unwrap(), twice);
        assert!(service.shutdown().await.unwrap().success());
    })
    .await
    .unwrap();
}

/// The result shows at most 60 numbered lines, then how to read the rest.
#[tokio::test]
async fn a_long_change_shows_its_first_60_lines_and_how_to_read_the_rest() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let before: String = (1..=200).map(|line| format!("l{line:03}\n")).collect();
        let (root, service) = service_with(&[("big.txt", &before)]).await;
        let replace: String = (50..=150).map(|line| format!("n{line:03}\n")).collect();
        let blocks = format!("<<<<<<< SEARCH\nl050\n.......\nl150\n=======\n{replace}>>>>>>> REPLACE\n");
        let (result, output, _) = call(
            service.client(),
            root.path().to_str().unwrap(),
            "file.edit",
            serde_json::json!({"path": "big.txt", "blocks": blocks}),
        )
        .await;
        assert_eq!(result.exit_code, 0, "{result:?}");
        let shown: String = (49..=108)
            .map(|line| {
                let text = if (50..=150).contains(&line) { format!("n{line:03}") } else { format!("l{line:03}") };
                format!("{line:>4}  {text}\n")
            })
            .collect();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!(
                "Edited big.txt (+101 \u{2212}101)\n{shown}[\u{2026} 43 more lines changed; read them: sed -n 109,151p big.txt]\n"
            )
        );
        assert!(service.shutdown().await.unwrap().success());
    })
    .await
    .unwrap();
}

/// Changes within two lines of each other show as one piece; farther ones
/// are set apart by `--`, and a blank line shows as its number alone.
#[tokio::test]
async fn nearby_changes_show_as_one_piece_and_pieces_are_set_apart() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let before: String = (1..=20).map(|line| if line == 9 { "\n".to_owned() } else { format!("l{line:02}\n") }).collect();
        let (root, service) = service_with(&[("pieces.txt", &before)]).await;
        // Lines 3 and 7 change: their shown lines, 2-4 and 6-8, lie one
        // line apart, so lines 2-8 are one piece. Line 15 is another.
        let blocks = "<<<<<<< SEARCH\nl03\n=======\nL03\n>>>>>>> REPLACE\n\
                      <<<<<<< SEARCH\nl07\n=======\nL07\n>>>>>>> REPLACE\n\
                      <<<<<<< SEARCH\nl15\n=======\nL15\n>>>>>>> REPLACE\n";
        let (result, output, _) = call(
            service.client(),
            root.path().to_str().unwrap(),
            "file.edit",
            serde_json::json!({"path": "pieces.txt", "blocks": blocks}),
        )
        .await;
        assert_eq!(result.exit_code, 0, "{result:?}");
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "Edited pieces.txt (+3 \u{2212}3)\n   2  l02\n   3  L03\n   4  l04\n   5  l05\n   6  l06\n   7  L07\n   8  l08\n\
             --\n  14  l14\n  15  L15\n  16  l16\n"
        );
        // A blank line next to a change shows as its number alone.
        let (result, output, _) = call(
            service.client(),
            root.path().to_str().unwrap(),
            "file.edit",
            serde_json::json!({"path": "pieces.txt", "blocks": "<<<<<<< SEARCH\nl10\n=======\nL10\n>>>>>>> REPLACE\n"}),
        )
        .await;
        assert_eq!(result.exit_code, 0, "{result:?}");
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "Edited pieces.txt (+1 \u{2212}1)\n   9\n  10  L10\n  11  l11\n"
        );
        assert!(service.shutdown().await.unwrap().success());
    })
    .await
    .unwrap();
}

/// A unified diff applies where its hunks' context and removed lines match,
/// whatever line counts its headers give, as `git apply --recount` reads
/// them; a hunk that matches nowhere changes no file.
#[tokio::test]
async fn a_patch_applies_its_hunks_where_their_lines_match() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let (root, service) = service_with(&[
            ("counts.txt", "one\ntwo\nthree\nfour\nfive\n"),
            ("bare.txt", "keep\nold\nkeep\n"),
            ("twice.txt", "a\nx\nb\nc\na\nx\nb\nd\n"),
            ("first.txt", "first\nline\n"),
            ("second.txt", "second\n"),
            ("blank.txt", "start\n\nend\n"),
            ("dashes.sql", "-- note\nselect 1;\n"),
        ])
        .await;
        let cwd = root.path().to_str().unwrap();
        let read = |name: &str| std::fs::read_to_string(root.path().join(name)).unwrap();
        let patch = |diff: &str| {
            call(
                service.client(),
                cwd,
                "file.patch",
                serde_json::json!({"patch": diff}),
            )
        };

        // The header says 5 old and 8 new lines; the hunk has 3 and 5.
        let (result, _, error) = patch(
            "--- a/counts.txt\n+++ b/counts.txt\n@@ -2,5 +2,8 @@\n two\n-three\n+3a\n+3b\n+3c\n four\n",
        )
        .await;
        assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&error));
        assert_eq!(read("counts.txt"), "one\ntwo\n3a\n3b\n3c\nfour\nfive\n");

        let (result, _, error) = patch("--- a/bare.txt\n+++ b/bare.txt\n@@\n keep\n-old\n+new\n").await;
        assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&error));
        assert_eq!(read("bare.txt"), "keep\nnew\nkeep\n");

        // The hunk matches at lines 1 and 5; its header's line 4 is nearer
        // the second.
        let (result, _, error) = patch("--- a/twice.txt\n+++ b/twice.txt\n@@ -4,3 +4,3 @@\n a\n-x\n+y\n b\n").await;
        assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&error));
        assert_eq!(read("twice.txt"), "a\nx\nb\nc\na\ny\nb\nd\n");

        // An empty line in a hunk is an empty context line; the text after
        // a header's second @@ is git's, and blank lines after a hunk end it.
        let (result, _, error) = patch("--- a/blank.txt\n+++ b/blank.txt\n@@ -1,3 +1,3 @@ fn start\n start\n\n-end\n+END\n\n").await;
        assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&error));
        assert_eq!(read("blank.txt"), "start\n\nEND\n");

        // A removed `-- note` before an added `++ note` stays in its hunk:
        // no @@ follows them.
        let (result, _, error) = patch("--- a/dashes.sql\n+++ b/dashes.sql\n@@\n--- note\n+++ note\n select 1;\n").await;
        assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&error));
        assert_eq!(read("dashes.sql"), "++ note\nselect 1;\n");

        // The second file's hunk matches nowhere: the patch names it, and
        // the first file, whose hunk applies, is left as it was.
        let (result, _, _) = patch(
            "--- a/first.txt\n+++ b/first.txt\n@@ -1,4 +1,4 @@\n-first\n+changed\n line\n--- a/second.txt\n+++ b/second.txt\n@@ -1 +1 @@\n-absent\n+changed\n",
        )
        .await;
        assert_eq!(result.exit_code, 1);
        assert!(
            message(&result).contains("Patch does not apply to second.txt: hunk 1 (@@ -1 +1 @@) matches no lines"),
            "{}",
            message(&result)
        );
        assert_eq!(read("first.txt"), "first\nline\n");
        assert_eq!(read("second.txt"), "second\n");
        assert!(service.shutdown().await.unwrap().success());
    })
    .await
    .unwrap();
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

/// What `demi file view` handed to its job and wrote.
#[derive(Debug, Default)]
struct Viewed {
    stdout: Vec<u8>,
    stderr: String,
    media: Vec<(MediumFacts, Bytes)>,
}

impl OutputSink for Viewed {
    type Error = ServiceError;

    async fn stdout(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        self.stdout.extend_from_slice(&bytes);
        Ok(())
    }

    async fn stderr(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        self.stderr.push_str(&String::from_utf8_lossy(&bytes));
        Ok(())
    }

    async fn medium(&mut self, facts: MediumFacts, bytes: Bytes) -> Result<(), ServiceError> {
        self.media.push((facts, bytes));
        Ok(())
    }
}

/// A job's stdin: `chunks` in turn, then its end; none at all when it is
/// the job's own input, which must never be read.
struct Stdin {
    chunks: Vec<Bytes>,
    live: bool,
}

impl InputSource for Stdin {
    type Error = ServiceError;

    async fn next(&mut self) -> Result<Option<Bytes>, ServiceError> {
        assert!(!self.live, "the job's own input was read");
        Ok((!self.chunks.is_empty()).then(|| self.chunks.remove(0)))
    }
}

/// Runs `demi file view` with `paths` in `cwd` as a job's command whose
/// model, `model`, reads `viewable` in a tool result, or types nobody knows
/// when none, with `stdin`.
async fn view(
    client: &Client,
    cwd: &str,
    paths: &[&str],
    model: &str,
    viewable: Option<&[&str]>,
    stdin: Stdin,
) -> (u8, Viewed) {
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
        operation: "file.view".into(),
        invocation_id: "view".into(),
        command: "demi file view".into(),
        args: serde_json::json!({ "path": paths }),
        cwd: cwd.into(),
        env: BTreeMap::new(),
        live_input: Some(stdin.live),
        viewable: Some(Viewable {
            model: model.into(),
            media_types: viewable
                .map(|types| types.iter().map(|&media_type| media_type.to_owned()).collect()),
        }),
    };
    let (input, output) = client.invoke(&request).await.unwrap();
    let mut stdin = stdin;
    let mut viewed = Viewed::default();
    let completion = Exchange::new(input, output)
        .run(&mut stdin, &mut viewed)
        .await
        .unwrap();
    assert_eq!(completion.error, None);
    (completion.exit_code, viewed)
}

/// A 4 × 3 PNG.
fn png() -> Vec<u8> {
    let pixels = image::RgbaImage::from_pixel(4, 3, image::Rgba([1, 2, 3, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    pixels.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// The facts of a medium of `media_type`, `name`d when it is a document,
/// `size` pixels when an image.
fn facts(media_type: &str, name: Option<&str>, size: Option<(u32, u32)>) -> MediumFacts {
    MediumFacts {
        media_type: media_type.into(),
        name: name.map(str::to_owned),
        width: size.map(|(width, _)| width),
        height: size.map(|(_, height)| height),
        duration_ms: None,
    }
}

const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n%%EOF\n";

/// About 1 s here: the program starts and views twelve times.
///
/// Planted defects this catches: a view that writes the medium to stdout;
/// one that stops at the first path that fails; a text file, bytes that are
/// no medium, a type the model does not read, or more than 16 MiB handed
/// to the job; a PDF not recognized; stdin not read for `-` or no path; and
/// the job's own input read, which would wait until the job ends.
#[tokio::test]
async fn demi_file_view_hands_each_medium_to_the_job_and_names_what_it_cannot_show() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let (root, service) = service_with(&[("notes.txt", "hello\n")]).await;
        let cwd = root.path().to_str().unwrap();
        std::fs::write(root.path().join("a.png"), png()).unwrap();
        std::fs::write(root.path().join("report.pdf"), PDF).unwrap();
        std::fs::write(root.path().join("data.bin"), [0u8, 255, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]).unwrap();
        let mut big = b"\x89PNG\r\n\x1a\n".to_vec();
        big.resize(17 * 1024 * 1024, 1);
        std::fs::write(root.path().join("capture.png"), big).unwrap();
        let client = service.client();
        let both = Some(&["image/png", "application/pdf"][..]);
        let none = || Stdin { chunks: Vec::new(), live: false };

        // Paths in order: the media go to the job, nothing to stdout, a
        // line for each that cannot be shown, and the others still shown.
        let paths = ["a.png", "notes.txt", "data.bin", "missing.png", "report.pdf", "capture.png"];
        let (code, viewed) = view(client, cwd, &paths, "test-model", both, none()).await;
        assert_eq!(code, 1);
        assert_eq!(viewed.stdout, b"");
        // Each with the facts its header gives: the image's size, the
        // document's file name.
        assert_eq!(
            viewed.media,
            [
                (facts("image/png", None, Some((4, 3))), Bytes::from(png())),
                (facts("application/pdf", Some("report.pdf"), None), Bytes::from_static(PDF)),
            ]
        );
        assert_eq!(
            viewed.stderr,
            "demi file view: notes.txt: a text file; read it with cat notes.txt\n\
             demi file view: data.bin: not an image, a video or a PDF (13 bytes)\n\
             demi file view: missing.png: No such file or directory\n\
             demi file view: capture.png: 17.0 MiB; a medium is at most 16 MiB\n"
        );

        // A type the model does not read in a tool result names the model,
        // and so does a model whose types are not known, which is no no.
        let (code, viewed) = view(client, cwd, &["report.pdf"], "deepseek-v4.1-flash", Some(&["image/png"]), none()).await;
        assert_eq!((code, viewed.media.len()), (1, 0));
        assert_eq!(
            viewed.stderr,
            "demi file view: report.pdf: this conversation's model, deepseek-v4.1-flash, does not read application/pdf in a tool result\n"
        );
        let (code, viewed) = view(client, cwd, &["a.png"], "deepseek-v4.1-flash", None, none()).await;
        assert_eq!((code, viewed.media.len()), (1, 0));
        assert_eq!(
            viewed.stderr,
            "demi file view: a.png: it is not known which files this conversation's model, deepseek-v4.1-flash, reads; its provider entry can name them\n"
        );

        // A pipe: stdin when no path is named or the path is -.
        for paths in [&[][..], &["-"][..]] {
            let piped = Stdin { chunks: vec![Bytes::from(png()[..20].to_vec()), Bytes::from(png()[20..].to_vec())], live: false };
            let (code, viewed) = view(client, cwd, paths, "test-model", both, piped).await;
            assert_eq!(
                (code, viewed.media, viewed.stderr.as_str()),
                (0, vec![(facts("image/png", None, Some((4, 3))), Bytes::from(png()))], "")
            );
        }
        let text = Stdin { chunks: vec![Bytes::from_static(b"plain words\n")], live: false };
        let (code, viewed) = view(client, cwd, &[], "test-model", both, text).await;
        assert_eq!((code, viewed.stderr.as_str()), (1, "demi file view: stdin: a text file; read it with cat stdin\n"));

        // The job's own input is never read: nothing waits for input.
        let live = Stdin { chunks: Vec::new(), live: true };
        let (code, viewed) = view(client, cwd, &[], "test-model", both, live).await;
        assert_eq!(code, 1);
        assert_eq!(
            viewed.stderr,
            "demi file view: no file named, and stdin is the job's input; name a file or pipe one in\n"
        );
        assert!(service.shutdown().await.unwrap().success());
    })
    .await
    .unwrap();
}
