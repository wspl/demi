//! The `demi file` group: every leaf runs a `file.*` operation of the
//! `demi.builtin` package beside the file, its arguments declared from the
//! `builtin-protocol` types (`commands.md` § File commands).

use demi_builtin_protocol::PACKAGE;
use demi_builtin_protocol::file::{CreateArgs, EditArgs, PatchArgs, ReadArgs};
use demi_command_tree::NativeOperation;
use demi_shell::{GroupBuilder, LeafBuilder};


pub(crate) fn file_group() -> GroupBuilder {
    GroupBuilder::new(
        "file",
        "Read, create, edit, and patch workspace files (text, images, and video).",
    )
    .leaf(
        leaf(
            "read",
            "Read a file. Text files print as text; image and video files are shown to you as viewable media. Output is the raw file bytes, so it also pipes cleanly into other commands (e.g. ffmpeg).",
        )
        .input::<ReadArgs>()
        .positionals(["path"])
        .success_output(
            "writes the raw file bytes to stdout; an image or video result is presented to you as viewable media",
        )
        .failure_output(
            "writes the reason to stderr and exits non-zero if the path is missing or unreadable",
        ),
    )
    .leaf(
        leaf("create", "Create a new file. Fails if the file exists.")
            .input::<CreateArgs>()
            .positionals(["path"])
            .stdin_field("content")
            .success_output("writes \"Created <path>\" to stdout")
            .failure_output(
                "writes the reason to stderr and exits non-zero without overwriting existing files",
            ),
    )
    .leaf(
        leaf("edit", "Replace exact text in an existing file.")
            .input::<EditArgs>()
            .positionals(["path"])
            .success_output("writes \"Edited <path>\" to stdout")
            .failure_output(
                "writes no-match, ambiguous-match, or write errors to stderr and exits non-zero without partial writes",
            ),
    )
    .leaf(
        leaf("patch", "Apply a unified diff patch to one or more files.")
            .input::<PatchArgs>()
            .stdin_field("patch")
            .success_output("writes \"Patched <n> file(s)\" to stdout")
            .failure_output(
                "writes parse, validation, or write errors to stderr and exits non-zero after rolling back partial writes when possible",
            ),
    )
}

/// A leaf running `file.<name>`.
fn leaf(name: &str, summary: &str) -> LeafBuilder {
    LeafBuilder::native(
        name,
        summary,
        NativeOperation {
            package: PACKAGE.into(),
            operation: format!("file.{name}"),
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::command_line::{demi, help, parse};

    #[test]
    fn file_bodies_come_only_from_stdin_and_any_word_is_a_file_name() {
        let (_, root) = demi();
        let body = "$HOME `echo unsafe` \"quotes\"\n";
        let created = parse(&root, &["file", "create", "note.txt"], Some(body)).unwrap();
        assert_eq!(created.values["content"], body);
        // A body option is refused before anything runs.
        for line in [
            &["file", "create", "forbidden.txt", "--content"][..],
            &["file", "create", "forbidden.txt", "--content", "body"][..],
            &["file", "patch", "--patch", "not a patch"][..],
        ] {
            let refused = parse(&root, line, None).unwrap_err().to_string();
            assert!(refused.contains("only from stdin. Remove --"), "{refused}");
        }
        // Help is a flag, so no word is reserved: a file named "prompt" is
        // just a file.
        let named = parse(&root, &["file", "create", "prompt"], Some("not help\n")).unwrap();
        assert!(!named.help);
        assert_eq!(named.values["path"], "prompt");
        assert!(
            parse(&root, &["file", "read", "--help"], None)
                .unwrap()
                .help
        );
        let read = help(&root, &["file", "read"]);
        assert!(read.starts_with("demi file read: Read a file."), "{read}");
        assert!(
            read.contains("<path> (required) - File path to read"),
            "{read}"
        );
        assert!(read.contains("shown to you as viewable media"), "{read}");
    }
}
