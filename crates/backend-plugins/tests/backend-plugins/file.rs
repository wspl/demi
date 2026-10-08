//! `demi file` as the model runs it (`commands.md` § File commands): each
//! script is one `shell_exec` in the conversation's shell on a real runner,
//! whose `file` commands run in the `demi.file` package the workspace
//! built.

use demi_agent_tools::testing::{field, shown_output};
use demi_provider_common::testing::ScriptedRuntime;

use crate::support::{Fixture, scripts, turn, within};

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
            shown_output(result).contains(text),
            "{text:?} is not in:\n{result}"
        );
    }
}

/// A PNG signature and three more bytes: not text, and not a whole image.
const PNG: [u8; 11] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe,
];

// Several seconds: nine scripts run a shell job each, and the first
// `demi file` starts the `demi.file` service.
#[tokio::test(flavor = "local")]
async fn demi_file_reads_and_creates_files_in_and_beyond_the_workspace() {
    within(async {
        let (fixture, results) = run(
            &[
                "demi file edit <<'EOF'\nnote.txt\n<<<<<<< SEARCH\n=======\nhello world\n>>>>>>> REPLACE\nEOF",
                "demi file read note.txt",
                "demi file edit <<'EOF'\nnote.txt\n<<<<<<< SEARCH\n=======\nagain\n>>>>>>> REPLACE\nEOF",
                "demi file edit <<'EOF'\nsrc/foo.txt\n<<<<<<< SEARCH\n=======\nhello\n>>>>>>> REPLACE\nEOF\ncat src/foo.txt",
                // An unquoted heredoc expands the path line.
                "demi file edit <<EOF\n$(cd .. && pwd)/absolute.txt\n<<<<<<< SEARCH\n=======\nnope\n>>>>>>> REPLACE\nEOF",
                "demi file edit <<'EOF'\n../relative.txt\n<<<<<<< SEARCH\n=======\nnope\n>>>>>>> REPLACE\nEOF",
                "demi file read shot.png",
                "demi file read shot.png | wc -c",
                "demi file --help && demi file edit --help",
            ],
            |workspace| std::fs::write(format!("{workspace}/shot.png"), PNG).unwrap(),
        )
        .await;
        assert_exit(&results[0], "0");
        assert_eq!(shown_output(&results[0]), "Created note.txt (1 line)\n");
        assert_eq!(shown_output(&results[1]), "hello world\n");
        // An existing file stays as it is.
        assert_exit(&results[2], "1");
        assert_eq!(shown_output(&results[3]), "Created src/foo.txt (1 line)\nhello\n");
        for result in &results[4..6] {
            assert_exit(result, "0");
        }
        // Binary stdout reaches the model as its size and the command that
        // saves its bytes, and pipes as bytes.
        assert_exit(&results[6], "0");
        assert_shows(
            &results[6],
            &["<binary stdout: 11 bytes>\n", "save it: demi shell output "],
        );
        assert_eq!(shown_output(&results[7]).trim(), "11");
        assert_shows(
            &results[8],
            &[
                "demi file edit",
                "Created <path> (<n> lines)",
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

// Several seconds: thirteen scripts run a shell job each, and the first
// `demi file` starts the `demi.file` service. The files the edits and the
// failed patch start from are written by the test, and it reads what they
// leave itself, so every job runs a command under test.
#[tokio::test(flavor = "local")]
async fn demi_file_edit_and_patch_change_what_they_name_whole_or_not_at_all() {
    within(async {
        let (fixture, results) = run(
            &[
                // Edits replace one exact match.
                "demi file edit file.txt --old two --new changed",
                "demi file edit file.txt --old two --new changed --occurrence 2 && cat file.txt",
                "demi file edit context.txt --old target --new changed --context 2",
                "cat context.txt && demi file edit context.txt --old target --new changed --context 3 && cat context.txt",
                "demi file edit empty-old.txt --old \"\" --new changed",
                // Blocks for two files in one quoted heredoc pass quotes, `$`
                // and backslashes as they are, and a REPLACE's last empty
                // line; the command after `&&` runs once both are written.
                QUOTED_EDIT,
                // With --old, an edit reads no stdin: the loop keeps its
                // input for its next turn.
                "printf 'loop-a.txt\\nloop-b.txt\\n' | while read f; do demi file edit \"$f\" --old one --new two; done; cat loop-a.txt loop-b.txt",
                // Patches apply whole unified diffs.
                "cat > patch.txt <<'EOF'\none\ntwo\nEOF\ndemi file patch <<'PATCH' && cat patch.txt\n--- a/patch.txt\n+++ b/patch.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+three\nPATCH",
                "cat > timed.txt <<'EOF'\nold\nEOF\ndemi file patch <<'PATCH' && cat timed.txt\n--- a/timed.txt 2026-06-17 00:00:00.000000000 +0800\n+++ b/timed.txt 2026-06-17 00:00:01.000000000 +0800\n@@ -1 +1 @@\n-old\n+new\nPATCH",
                "cat > existing.txt <<'EOF'\none\nEOF\ndemi file patch <<'PATCH' && cat existing.txt nested/new.txt\n--- a/existing.txt\n+++ b/existing.txt\n@@ -1 +1 @@\n-one\n+changed\n--- /dev/null\n+++ b/nested/new.txt\n@@ -0,0 +1,2 @@\n+new\n+file\nPATCH",
                "cat > doomed.txt <<'EOF'\nremove\nEOF\ndemi file patch <<'PATCH' && test ! -e doomed.txt && echo gone\n--- a/doomed.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-remove\nPATCH",
                "demi file patch <<'PATCH'\n--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n--- a/second.txt\n+++ b/second.txt\n@@ -1 +1 @@\n-wrong\n+changed\nPATCH",
                "cat > inside.txt <<'EOF'\ninside\nEOF\noutside=\"$(cd .. && pwd)/outside.txt\"\ndemi file patch <<PATCH && cat inside.txt\n--- a/inside.txt\n+++ b/inside.txt\n@@ -1 +1 @@\n-inside\n+changed\n--- /dev/null\n+++ $outside\n@@ -0,0 +1 @@\n+outside\nPATCH",
            ],
            |workspace| {
                for (name, content) in [
                    ("file.txt", "one\ntwo\ntwo\n"),
                    ("context.txt", "target\nmiddle\ntarget\n"),
                    ("empty-old.txt", "content\n"),
                    ("quoted.js", QUOTED_BEFORE),
                    ("loop-a.txt", "one\n"),
                    ("loop-b.txt", "one\n"),
                    ("first.txt", "first\n"),
                    ("second.txt", "second\n"),
                ] {
                    std::fs::write(format!("{workspace}/{name}"), content).unwrap();
                }
            },
        )
        .await;
        let read = |name: &str| std::fs::read_to_string(format!("{}/{name}", fixture.workspace)).unwrap();
        assert_exit(&results[0], "1");
        assert_shows(&results[0], &["Multiple matches in file.txt; specify --occurrence or --context"]);
        assert_eq!(shown_output(&results[1]), "Edited file.txt (+1 \u{2212}1)\n   2  two\n   3  changed\none\ntwo\nchanged\n");
        assert_exit(&results[2], "1");
        assert_shows(
            &results[2],
            &["Context line 2 is ambiguous", "occurrence 1 at line 1", "occurrence 2 at line 3"],
        );
        assert_eq!(
            shown_output(&results[3]),
            "target\nmiddle\ntarget\nEdited context.txt (+1 \u{2212}1)\n   2  middle\n   3  changed\ntarget\nmiddle\nchanged\n"
        );
        assert_exit(&results[4], "1");
        assert_shows(&results[4], &["Invalid command arguments: \"old\" is shorter than 1 character"]);
        assert_eq!(read("empty-old.txt"), "content\n");
        assert_eq!(shown_output(&results[5]), QUOTED_SHOWN);
        assert_eq!(read("quoted.js"), QUOTED_AFTER);
        assert_eq!(read("notes/new.md"), QUOTED_CREATED);
        assert_eq!(
            shown_output(&results[6]),
            "Edited loop-a.txt (+1 \u{2212}1)\n   1  two\nEdited loop-b.txt (+1 \u{2212}1)\n   1  two\ntwo\ntwo\n"
        );

        assert_eq!(shown_output(&results[7]), "Patched 1 file(s)\none\nthree\n");
        assert_eq!(shown_output(&results[8]), "Patched 1 file(s)\nnew\n");
        assert_eq!(
            shown_output(&results[9]),
            "Patched 2 file(s)\nchanged\nnew\nfile\n"
        );
        assert_eq!(shown_output(&results[10]), "Patched 1 file(s)\ngone\n");
        // One file that does not apply leaves every file as it was.
        assert_exit(&results[11], "1");
        assert_shows(&results[11], &["Patch does not apply to second.txt"]);
        assert_eq!(read("first.txt"), "first\n");
        assert_eq!(shown_output(&results[12]), "Patched 2 file(s)\nchanged\n");
        assert_eq!(
            std::fs::read_to_string(format!("{}/outside.txt", fixture.runner.home())).unwrap(),
            "outside\n"
        );
        fixture.stop().await;
    })
    .await;
}

/// A file whose text a shell would change unless quoted: quotes, `$` and
/// backslashes.
const QUOTED_BEFORE: &str = r#"// head
const greeting = "hello";
const path = 'C:\temp';
// tail
"#;

/// The edit of [`QUOTED_BEFORE`] and a new file, as the model writes it.
const QUOTED_EDIT: &str = r#"demi file edit <<'EOF' && cat notes/new.md
quoted.js
<<<<<<< SEARCH
const greeting = "hello";
const path = 'C:\temp';
=======
const greeting = "it's $HOME, \"quoted\"";
const path = 'C:\new\temp';

>>>>>>> REPLACE

notes/new.md
<<<<<<< SEARCH
=======
It's $HOME's \"note\"
>>>>>>> REPLACE
EOF"#;

const QUOTED_AFTER: &str = r#"// head
const greeting = "it's $HOME, \"quoted\"";
const path = 'C:\new\temp';

// tail
"#;

const QUOTED_CREATED: &str = r#"It's $HOME's \"note\"
"#;

/// What [`QUOTED_EDIT`] shows: each change as the file now reads, then the
/// new file the command after it prints.
const QUOTED_SHOWN: &str = concat!(
    "Edited quoted.js (+3 \u{2212}2)\n",
    "   1  // head\n",
    "   2  const greeting = \"it's $HOME, \\\"quoted\\\"\";\n",
    "   3  const path = 'C:\\new\\temp';\n",
    "   4\n",
    "   5  // tail\n",
    "Created notes/new.md (1 line)\n",
    "It's $HOME's \\\"note\\\"\n",
);
