//! The `demi file` group: every leaf runs a `file.*` operation of the
//! `demi.file` package beside the file, its arguments declared from the
//! `command-package-file-protocol` types (`commands.md` § File commands).

use demi_command_declarations::NativeOperation;
use demi_command_package_file_protocol::{CreateArgs, EditArgs, PACKAGE, PatchArgs, ReadArgs};
use demi_host_interface::{GroupBuilder, LeafBuilder};

pub(crate) fn file_group() -> GroupBuilder {
    GroupBuilder::new(
        "file",
        "Read, create, edit, and patch workspace files (text, images, and video).",
    )
    .leaf(
        leaf(
            "read",
            "Read a file. Text files print as text; an image or video file is shown to you as viewable media, and several reads in one script show each in order. Into a file or a pipe it writes the raw file bytes, so it also pipes cleanly into other commands (e.g. ffmpeg).",
        )
        .input::<ReadArgs>()
        .positionals(["path"])
        .success_output(
            "writes the raw file bytes to stdout; an image or video file of at most 16 MiB is returned as a medium",
        )
        .media()
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
