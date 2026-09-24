//! `demi file` as the model runs it (`commands.md` § File commands): each
//! script is one `shell_exec` in the conversation's shell on a real runner,
//! whose `file` commands run in the `demi.builtin` package the workspace
//! built.

use demi_provider::testing::ScriptedRuntime;

use crate::support::{Fixture, field, preview, scripts, turn, within};

/// The results of running `scripts` in one message, with `prepare` run on
/// the workspace first; the fixture stays for the test's own checks.
async fn run(scripts_to_run: &[&str], prepare: impl FnOnce(&str)) -> (Fixture, Vec<String>) {
    let (turns, recorded) = scripts(&[scripts_to_run]);
    let fixture = Fixture::start(&ScriptedRuntime::new(turns)).await;
    prepare(&fixture.workspace);
    let mut client = fixture.opened().await;
    turn(&mut client, "message-1", "Work on the files.").await;
    let results = recorded.borrow().clone();
    assert_eq!(results.len(), scripts_to_run.len(), "{results:#?}");
    (fixture, results)
}

fn assert_exit(result: &str, code: &str) {
    assert!(result.starts_with("status: exited\n"), "{result}");
    assert_eq!(field(result, "exitCode"), code, "{result}");
}

fn assert_shows(result: &str, texts: &[&str]) {
    for text in texts {
        assert!(
            preview(result).contains(text),
            "{text:?} is not in:\n{result}"
        );
    }
}

/// A PNG signature and three more bytes: not text, and not a whole image.
const PNG: [u8; 11] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe,
];

#[tokio::test(flavor = "local")]
async fn demi_file_reads_and_creates_files_in_and_beyond_the_workspace() {
    within(async {
        let (fixture, results) = run(
            &[
                "demi file create note.txt <<'EOF'\nhello world\nEOF",
                "demi file read note.txt",
                "demi file create note.txt <<'EOF'\nagain\nEOF",
                "demi file create src/foo.txt <<'EOF'\nhello\nEOF\ncat src/foo.txt",
                "demi file create \"$(cd .. && pwd)/absolute.txt\" <<'EOF'\nnope\nEOF",
                "demi file create ../relative.txt <<'EOF'\nnope\nEOF",
                "demi file read shot.png",
                "demi file read shot.png | wc -c",
                "demi --help",
            ],
            |workspace| std::fs::write(format!("{workspace}/shot.png"), PNG).unwrap(),
        )
        .await;
        assert_exit(&results[0], "0");
        assert_eq!(preview(&results[0]), "Created note.txt\n");
        assert_eq!(preview(&results[1]), "hello world\n");
        // An existing file stays as it is.
        assert_exit(&results[2], "1");
        assert_eq!(preview(&results[3]), "Created src/foo.txt\nhello\n");
        for result in &results[4..6] {
            assert_exit(result, "0");
        }
        // Binary stdout reaches the model as its size and where its bytes
        // are, and pipes as bytes.
        assert_exit(&results[6], "0");
        assert_shows(
            &results[6],
            &[
                "<binary stdout: 11 bytes; raw bytes at ",
                "the raw bytes remain readable at ",
            ],
        );
        assert_eq!(preview(&results[7]).trim(), "11");
        assert_shows(
            &results[8],
            &[
                "demi file create",
                "Success output: writes \"Created <path>\" to stdout",
                "shown to you as viewable media",
            ],
        );

        let home = fixture.runner.home().to_owned();
        let read = |path: String| std::fs::read_to_string(path).unwrap();
        assert_eq!(
            read(format!("{}/note.txt", fixture.workspace)),
            "hello world\n"
        );
        assert_eq!(read(format!("{home}/absolute.txt")), "nope\n");
        assert_eq!(read(format!("{home}/relative.txt")), "nope\n");
        fixture.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "local")]
async fn demi_file_edit_and_patch_change_what_they_name_whole_or_not_at_all() {
    within(async {
        let (fixture, results) = run(
            &[
                // Edits replace one exact match.
                "demi file create file.txt <<'EOF'\none\ntwo\ntwo\nEOF",
                "demi file edit file.txt --old two --new changed",
                "demi file edit file.txt --old two --new changed --occurrence 2 && cat file.txt",
                "demi file create context.txt <<'EOF'\ntarget\nmiddle\ntarget\nEOF",
                "demi file edit context.txt --old target --new changed --context 2",
                "cat context.txt && demi file edit context.txt --old target --new changed --context 3 && cat context.txt",
                "demi file create empty-old.txt <<'EOF'\ncontent\nEOF",
                "demi file edit empty-old.txt --old \"\" --new changed",
                "cat empty-old.txt",
                // Patches apply whole unified diffs.
                "demi file create patch.txt <<'EOF'\none\ntwo\nEOF\ndemi file patch <<'PATCH' && cat patch.txt\n--- a/patch.txt\n+++ b/patch.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+three\nPATCH",
                "demi file create timed.txt <<'EOF'\nold\nEOF\ndemi file patch <<'PATCH' && cat timed.txt\n--- a/timed.txt 2026-06-17 00:00:00.000000000 +0800\n+++ b/timed.txt 2026-06-17 00:00:01.000000000 +0800\n@@ -1 +1 @@\n-old\n+new\nPATCH",
                "demi file create existing.txt <<'EOF'\none\nEOF\ndemi file patch <<'PATCH' && cat existing.txt nested/new.txt\n--- a/existing.txt\n+++ b/existing.txt\n@@ -1 +1 @@\n-one\n+changed\n--- /dev/null\n+++ b/nested/new.txt\n@@ -0,0 +1,2 @@\n+new\n+file\nPATCH",
                "demi file create doomed.txt <<'EOF'\nremove\nEOF\ndemi file patch <<'PATCH' && test ! -e doomed.txt && echo gone\n--- a/doomed.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-remove\nPATCH",
                "demi file create first.txt <<'EOF'\nfirst\nEOF\ndemi file create second.txt <<'EOF'\nsecond\nEOF",
                "demi file patch <<'PATCH'\n--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n--- a/second.txt\n+++ b/second.txt\n@@ -1 +1 @@\n-wrong\n+changed\nPATCH",
                "cat first.txt",
                "demi file create inside.txt <<'EOF'\ninside\nEOF\noutside=\"$(cd .. && pwd)/outside.txt\"\ndemi file patch <<PATCH && cat inside.txt\n--- a/inside.txt\n+++ b/inside.txt\n@@ -1 +1 @@\n-inside\n+changed\n--- /dev/null\n+++ $outside\n@@ -0,0 +1 @@\n+outside\nPATCH",
            ],
            |_| {},
        )
        .await;
        assert_exit(&results[1], "1");
        assert_shows(&results[1], &["Multiple matches in file.txt; specify --occurrence or --context"]);
        assert_eq!(preview(&results[2]), "Edited file.txt\none\ntwo\nchanged\n");
        assert_exit(&results[4], "1");
        assert_shows(
            &results[4],
            &["Context line 2 is ambiguous", "occurrence 1 at line 1", "occurrence 2 at line 3"],
        );
        assert_eq!(
            preview(&results[5]),
            "target\nmiddle\ntarget\nEdited context.txt\ntarget\nmiddle\nchanged\n"
        );
        assert_exit(&results[7], "1");
        assert_shows(&results[7], &["Invalid command arguments: \"\" is shorter than 1 character"]);
        assert_eq!(preview(&results[8]), "content\n");

        assert_eq!(preview(&results[9]), "Created patch.txt\nPatched 1 file(s)\none\nthree\n");
        assert_eq!(preview(&results[10]), "Created timed.txt\nPatched 1 file(s)\nnew\n");
        assert_eq!(
            preview(&results[11]),
            "Created existing.txt\nPatched 2 file(s)\nchanged\nnew\nfile\n"
        );
        assert_eq!(preview(&results[12]), "Created doomed.txt\nPatched 1 file(s)\ngone\n");
        // One file that does not apply leaves every file as it was.
        assert_exit(&results[14], "1");
        assert_shows(&results[14], &["Patch does not apply to second.txt"]);
        assert_eq!(preview(&results[15]), "first\n");
        assert_eq!(preview(&results[16]), "Created inside.txt\nPatched 2 file(s)\nchanged\n");
        assert_eq!(
            std::fs::read_to_string(format!("{}/outside.txt", fixture.runner.home())).unwrap(),
            "outside\n"
        );
        fixture.stop().await;
    })
    .await;
}
