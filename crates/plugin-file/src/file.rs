//! The `demi file` group: every leaf runs a `file.*` operation of the
//! `demi.file` package beside the file, its arguments declared from the
//! `command-package-file-protocol` types (`commands.md` § File commands).

use demi_command_declarations::NativeOperation;
use demi_command_package_file_protocol::{CreateArgs, EditArgs, PACKAGE, PatchArgs, ReadArgs};
use demi_host_interface::{GroupBuilder, LeafBuilder};

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Edits, patches and creates the task's files exactly, so the user can review each change, and shows you images and videos. Use it for every change to a file and whenever you need to see an image; plain shell tools are fine for looking around.";

/// The help of `demi file edit`'s stdin: the blocks, with an example the
/// model can copy. Its lines start at the margin, since a marker is a whole
/// line and indentation copied with it would make it text.
const EDIT_BLOCKS: &str = "SEARCH/REPLACE blocks. Each SEARCH is whole lines copied exactly from the file and must match one place; its REPLACE takes their place, and an empty one deletes them. Several blocks apply together or not at all. For a one-line change, pass --old and --new instead of stdin. Pass blocks in a quoted heredoc, so quotes, $ and backslashes need no escaping, for example:
demi file edit src/slugify.mjs <<'EOF'
<<<<<<< SEARCH
export function slugify(text) {
  return text.toLowerCase().replace(/\\s+/g, '-');
}
=======
export function slugify(text) {
  return text.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-');
}
>>>>>>> REPLACE
EOF";

pub(crate) fn file_group() -> GroupBuilder {
    GroupBuilder::new(
        "file",
        "Read, create, edit, and patch workspace files (text, images, and video).",
    )
    .index_entry(ENTRY)
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
        leaf(
            "edit",
            "Replace text in an existing file: SEARCH/REPLACE blocks on stdin, or --old and --new for one line.",
        )
        .input::<EditArgs>()
        .positionals(["path"])
        .stdin_field("blocks")
        .describe("blocks", EDIT_BLOCKS)
        .describe(
            "old",
            "Exact text to replace, without stdin; with several matches, --occurrence or --context chooses one",
        )
        .success_output("writes \"Edited <path>\" to stdout")
        .failure_output(
            "names a block that matches nowhere with the file's closest lines, or one that matches several places with each place's lines, and exits non-zero without changing the file",
        ),
    )
    .leaf(
        leaf("patch", "Apply a unified diff patch to one or more files.")
            .input::<PatchArgs>()
            .stdin_field("patch")
            .describe(
                "patch",
                "A unified diff, as git diff writes it. A hunk's header line counts may be off, and a bare @@ header works: each hunk goes where its context and removed lines match, the only place or the one nearest its header's line.",
            )
            .success_output("writes \"Patched <n> file(s)\" to stdout")
            .failure_output(
                "names the file and hunk that match nowhere and exits non-zero without changing any file",
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
