//! The `demi file` group: every leaf runs a `file.*` operation of the
//! `demi.file` package beside the file, its arguments declared from the
//! `command-package-file-protocol` types (`commands.md` § File commands).

use demi_command_declarations::NativeOperation;
use demi_command_package_file_protocol::{EditArgs, PACKAGE, PatchArgs, ReadArgs};
use demi_host_interface::{GroupBuilder, LeafBuilder};

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index): when to use it, and the shape of a call, since a
/// model takes up a tool by the example it has seen. Its example starts at
/// the margin, as `demi file edit`'s help says why.
const ENTRY: &str = "Use it whenever you change the task's files, so each change is exact and the user sees it; `read` shows you an image or video. One `edit` creates and changes several files, then you build:
demi file edit <<'EOF' && cargo check
src/stream.rs
<<<<<<< SEARCH
=======
pub struct Stream;
>>>>>>> REPLACE
src/lib.rs
<<<<<<< SEARCH
mod socket;
=======
mod socket;
mod stream;
>>>>>>> REPLACE
EOF";

/// The help of `demi file edit`'s stdin: the blocks, with an example the
/// model can copy. Its lines start at the margin, since a marker is a whole
/// line and indentation copied with it would make it text.
const EDIT_BLOCKS: &str = r"SEARCH/REPLACE blocks, each after a line naming its file, or all for the path argument. A SEARCH is whole lines copied exactly from the file and must match one place; its REPLACE takes their place, and an empty REPLACE deletes them. An empty SEARCH creates its file, which must not exist. A SEARCH line of seven dots ....... stands for the lines between the ones around it: give the REPLACE as many to keep those lines, or none to replace them. Every block of every file applies, or none does. For a one-line change, pass the path, --old and --new instead. Pass blocks in a quoted heredoc, so quotes, $ and backslashes need no escaping, for example:
demi file edit <<'EOF' && cargo check
src/stream.rs
<<<<<<< SEARCH
=======
pub struct Stream;
>>>>>>> REPLACE

src/lib.rs
<<<<<<< SEARCH
mod socket;
=======
mod socket;
mod stream;
>>>>>>> REPLACE

src/socket.rs
<<<<<<< SEARCH
pub async fn serve(socket: Socket) {
.......
    send_frames(&socket).await;
}
=======
pub async fn serve(socket: Socket) {
.......
    stream::send(&socket).await;
}
>>>>>>> REPLACE
EOF";

pub(crate) fn file_group() -> GroupBuilder {
    GroupBuilder::new(
        "file",
        "Read, edit, and patch workspace files (text, images, and video).",
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
        leaf(
            "edit",
            "Change files: SEARCH/REPLACE blocks on stdin for one or several files, new ones included, or --old and --new for one line.",
        )
        .input::<EditArgs>()
        .positionals(["path"])
        .stdin_field("blocks")
        .stdin_unless(["old"])
        .describe("blocks", EDIT_BLOCKS)
        .describe(
            "old",
            "Exact text to replace in the path argument's file, without stdin; with several matches, --occurrence or --context chooses one",
        )
        .success_output(
            "a line for each file, \"Created <path> (<n> lines)\" or \"Edited <path> (+<added> −<removed>)\", and under an edited file each change as the file now reads, numbered, with a line of context on each side",
        )
        .failure_output(
            "names the file and block that match nowhere, with the file's closest lines, or that match several places, with each place's lines, and exits non-zero without changing any file",
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
