//! The `demi file` group: every leaf runs a `file.*` operation of the
//! `demi.file` package beside the file, its arguments declared from the
//! `command-package-file-protocol` types (`commands.md` § File commands).

use demi_command_declarations::NativeOperation;
use demi_command_package_file_protocol::{EditArgs, PACKAGE, PatchArgs, ViewArgs};
use demi_host_interface::{GroupBuilder, LeafBuilder};

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index): when to use it, and the shape of a call, since a
/// model takes up a tool by the example it has seen. Its example starts at
/// the margin, as `demi file edit`'s help says why.
const ENTRY: &str = "Use it whenever you change the task's files, so each change is exact and the user sees it, and whenever you want to see an image, a video or a PDF: `view` shows you one from a file or a pipe, as in `demi browser screenshot t1 | demi file view`. Read text with cat, sed -n or rg. One `edit` creates and changes several files, then you build:
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
const EDIT_BLOCKS: &str = r"SEARCH/REPLACE blocks, each after a line naming its file, or all for the path argument. A SEARCH is text copied exactly from the file, whitespace and indentation included: whole lines, or part of one, such as withinLimit( inside a line. It must occur once in the file; its REPLACE takes its place. A REPLACE with no lines deletes the SEARCH, and a SEARCH of whole lines with its last line ending, so deleted lines leave no blank one. A SEARCH of blank lines only is refused. An empty SEARCH creates its file, which must not exist. A SEARCH line of seven dots ....... stands for the lines between the ones around it, and the block then matches whole lines: give the REPLACE as many to keep those lines, or none to replace them. Every block of every file applies, or none does. For a one-line change, pass the path, --old and --new instead. Pass blocks in a quoted heredoc, so quotes, $ and backslashes need no escaping, for example:
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
        "Edit and patch workspace files, and view images, videos and PDFs.",
    )
    .index_entry(ENTRY)
    .leaf(
        leaf(
            "view",
            "Show yourself images, videos and PDFs, in order: each path, or stdin for - or no path, so demi file view a.png b.png shows both and demi browser screenshot t1 | demi file view shows the screenshot. Each one is attached to the result that reports the command's end, whatever its stdout is. Text is read with cat, sed -n or rg, not with this.",
        )
        .input::<ViewArgs>()
        .positionals(["path"])
        .success_output(
            "nothing on stdout; a line for each medium in the command's output, such as [image 1: image/png, 1280 × 720 px, 412000 bytes], and the medium attached to the result",
        )
        .media()
        .failure_output(
            "a line per path that cannot be shown, `demi file view: <path>: <reason>`, on stderr, such as a text file, bytes that are no image, video or PDF, a type this conversation's model does not read, or more than 16 MiB; the other paths are still shown, and the command exits 1",
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
            "names the file by its full path and the block whose SEARCH is not in the file, with the file's closest lines, or occurs more than once, with the line of each, and exits non-zero without changing any file",
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
