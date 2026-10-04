//! The media a job's declared commands return (`runtime.md` § Media a
//! command returns, `commands.md` § Return media) over a real runner: a
//! medium goes to the job, its line into stdout at its place, when the
//! command's stdout is the job's output, and is the command's whole stdout
//! otherwise; the backend has each one the job kept when the job ended.

use bytes::Bytes;
use demi_backend_remote_host::{
    RemoteShellEnvironment,
    testing::{FixtureOptions, NativeFixture, RunnerFixture},
};
use demi_host_interface::{
    Call, CommandSet, CommandState, CommandStatus, GroupBuilder, LeafBuilder, RpcError, RpcPort,
    TypedRpc,
};
use demi_shared_types::BlobRef;
use schemars::JsonSchema;
use serde_json::{Map, Value};

use crate::runner::{catalog, exited, native_leaf, run, shell_on};

#[derive(JsonSchema)]
#[schemars(deny_unknown_fields)]
#[expect(dead_code, reason = "the declaration needs only the schema")]
struct MediumArgs {
    /// Printed before the media
    before: Option<String>,
    /// How many media
    count: Option<u64>,
    /// Printed after the media
    after: Option<String>,
    /// Each medium's bytes
    size: Option<u64>,
    /// "text" for bytes that are no medium
    kind: Option<String>,
}

/// Medium `index` of the fixture's `medium` operation, 16 bytes long.
fn png(index: u8) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.resize(16, index);
    bytes
}

/// What the backend's `returned` handler returns as its medium.
fn returned() -> Bytes {
    Bytes::from(png(9))
}

/// Prints a line, returns the blob of [`returned`] and prints another, as a
/// handler in the backend does.
async fn return_medium(_: Call<Map<String, Value>>, port: RpcPort) -> Result<u8, RpcError> {
    port.stdout(b"rpc before\n".to_vec()).await?;
    port.medium(BlobRef::of(&returned())).await?;
    port.stdout(b"rpc after\n".to_vec()).await?;
    Ok(0)
}

/// A runner whose jobs run `demi medium`, the fixture's media; `demi
/// undeclared`, the same operation under a leaf that does not declare
/// media; and `demi returned`, a handler in the backend that returns one.
async fn media_runner() -> (RunnerFixture, RemoteShellEnvironment) {
    let native = NativeFixture::load();
    let mut commands = CommandSet::new();
    commands
        .register(
            GroupBuilder::new("demi", "Media probes.")
                .leaf(
                    native_leaf(&native, "medium", "medium")
                        .input::<MediumArgs>()
                        .media(),
                )
                .leaf(native_leaf(&native, "undeclared", "medium").input::<MediumArgs>())
                .leaf(
                    LeafBuilder::rpc("returned", "Return a medium from the backend.")
                        .media()
                        .bind(TypedRpc::new(return_medium)),
                ),
        )
        .unwrap();
    let selection = catalog(&native).select(&commands).unwrap();
    let fixture = RunnerFixture::start(FixtureOptions {
        commands,
        ..FixtureOptions::default()
    })
    .await;
    fixture.policy().put_blob(returned());
    let shell = shell_on(fixture.host(), &[], Some(selection));
    (fixture, shell)
}

/// The media an ended command returned: each one's number, type and bytes.
fn media(status: &CommandStatus) -> Vec<(u32, String, Vec<u8>)> {
    let CommandState::Exited { media, .. } = &status.state else {
        panic!("expected an exited command, got {:?}", status.state);
    };
    media
        .iter()
        .map(|medium| {
            let bytes = medium.bytes.as_ref().expect("the backend has the medium");
            (medium.number, medium.media_type.clone(), bytes.to_vec())
        })
        .collect()
}

/// The binary stdout of an ended command.
fn binary(status: &CommandStatus) -> Option<Vec<u8>> {
    let CommandState::Exited { binary_stdout, .. } = &status.state else {
        panic!("expected an exited command, got {:?}", status.state);
    };
    binary_stdout.as_ref().map(|binary| binary.bytes.to_vec())
}

fn png_medium(number: u32, index: u8) -> (u32, String, Vec<u8>) {
    (number, "image/png".to_owned(), png(index))
}

/// About 2.5 s here: six jobs one after another, each a login shell.
///
/// Planted defects this catches: a runner that names no `DEMI_JOB_OUTPUT`
/// (every medium goes elsewhere, so nothing is attached and the loop fails
/// on its second medium); one that sends every medium to the job (`>
/// shot.png` stays empty); one that lets a second medium with a stdout that
/// goes elsewhere through; and a line placed after the command's stdout
/// rather than where it returned the medium.
#[tokio::test(flavor = "local")]
async fn a_medium_goes_to_the_job_or_is_the_whole_stdout_by_where_the_stdout_goes() {
    let (fixture, shell) = media_runner().await;
    let home = fixture.home().to_owned();

    // Run on its own: the line among the command's text, the medium kept.
    let direct = run(&shell, "demi medium --before shot --after done").await;
    assert_eq!(exited(&direct), 0, "{}", direct.stderr.tail);
    assert_eq!(
        direct.stdout.delta,
        "shot\n[medium 1: image/png, 16 bytes]\ndone"
    );
    assert_eq!(media(&direct), [png_medium(1, 1)]);

    // Into a file: the file holds the bytes, and nothing is attached.
    let saved = run(&shell, "demi medium > shot.png").await;
    assert_eq!(exited(&saved), 0, "{}", saved.stderr.tail);
    assert_eq!(std::fs::read(format!("{home}/shot.png")).unwrap(), png(1));
    assert_eq!((saved.stdout.delta.as_str(), media(&saved)), ("", vec![]));

    // Into a pipe: the reader's stdout is the job's, here the bytes again,
    // a binary stdout; the medium itself is never the job's.
    let piped = run(&shell, "demi medium | cat").await;
    assert_eq!(exited(&piped), 0, "{}", piped.stderr.tail);
    assert_eq!((media(&piped), binary(&piped)), (vec![], Some(png(1))));

    // Two media and a stdout that carries one: the command fails, the file
    // stays empty.
    let two = run(&shell, "demi medium --count 2 > out.bin").await;
    assert_eq!(exited(&two), 1);
    assert_eq!(
        two.stderr.delta,
        "demi medium: returns 2 media, but its stdout is not the job's output and carries only one; run it once per medium, or let its stdout reach the job's output\n"
    );
    assert_eq!(std::fs::read(format!("{home}/out.bin")).unwrap(), b"");

    // A medium beside text in a stdout that goes elsewhere fails too.
    let beside = run(&shell, "demi medium --after more | cat").await;
    assert!(
        beside.stderr.delta.contains(
            "demi medium: a medium must be all of its stdout when its stdout is not the job's output"
        ),
        "{}",
        beside.stderr.delta
    );

    // A loop's body inherits the job's output: every medium, in its order.
    let looped = run(&shell, "for i in 1 2 3; do demi medium --count 1; done").await;
    assert_eq!(exited(&looped), 0, "{}", looped.stderr.tail);
    assert_eq!(
        looped.stdout.delta,
        "[medium 1: image/png, 16 bytes]\n[medium 2: image/png, 16 bytes]\n[medium 3: image/png, 16 bytes]\n"
    );
    assert_eq!(
        media(&looped),
        [png_medium(1, 1), png_medium(2, 1), png_medium(3, 1)]
    );
    fixture.stop().await;
}

/// About 1 s here: one job.
///
/// Planted defects this catches: a comparison by the kind of descriptor
/// rather than by its file (a pipeline's pipe and a redirected file would
/// read as the job's output); one that does not follow a copy of the job's
/// pipe (`exec 3>&1` would read as elsewhere); and an alias that does not
/// compare its own stdout (`xargs` would write the bytes).
#[tokio::test(flavor = "local")]
async fn a_commands_stdout_is_the_jobs_output_only_where_it_reaches_the_jobs_pipe() {
    let (fixture, shell) = media_runner().await;
    let script = [
        "demi medium --before direct",
        "{ demi medium --before group; }",
        "( demi medium --before subshell )",
        "demi medium --before background & wait",
        "exec 3>&1; demi medium --before copy >&3",
        "echo --before=alias | xargs demi medium",
        "demi medium | wc -c",
        "demi medium > /dev/null",
        "x=$(demi medium)",
        "cat <(demi medium) > /dev/null",
    ]
    .join("\n");
    let status = run(&shell, &script).await;
    assert_eq!(exited(&status), 0, "{}", status.stderr.tail);
    let lines: String = ["direct", "group", "subshell", "background", "copy", "alias"]
        .iter()
        .enumerate()
        .map(|(index, name)| format!("{name}\n[medium {}: image/png, 16 bytes]\n", index + 1))
        .collect();
    // The pipe took the bytes, and the null device and the substitutions
    // took theirs: none of them is a medium of the job.
    let (attached, piped) = status
        .stdout
        .delta
        .split_at(lines.len().min(status.stdout.delta.len()));
    assert_eq!((attached, piped.trim()), (lines.as_str(), "16"));
    assert_eq!(media(&status).len(), 6);
    fixture.stop().await;
}

/// About 1 s here: four jobs, one of them returning 80 MiB of media.
///
/// Planted defects this catches: an rpc medium placed where it arrives
/// rather than after the stdout bytes it followed; a medium the runner keeps
/// past its count or its bytes; and a check that lets a leaf without
/// `media`, or bytes that are no image or video, return one.
#[tokio::test(flavor = "local")]
async fn media_keep_their_place_their_order_their_bounds_and_their_checks() {
    let (fixture, shell) = media_runner().await;

    // A native and an rpc command of one job: each medium's line lies
    // between its command's two lines, numbered in the order they came.
    let both = run(&shell, "demi medium --before 'native before' --after 'native after'; echo; demi returned").await;
    assert_eq!(exited(&both), 0, "{}", both.stderr.tail);
    assert_eq!(
        both.stdout.delta,
        "native before\n[medium 1: image/png, 16 bytes]\nnative after\nrpc before\n[medium 2: image/png, 16 bytes]\nrpc after\n"
    );
    assert_eq!(
        media(&both),
        [png_medium(1, 1), (2, "image/png".to_owned(), returned().to_vec())]
    );

    // The 33rd medium is not kept: its place says so, and the command goes
    // on and succeeds.
    let many = run(&shell, "demi medium --count 33").await;
    assert_eq!(exited(&many), 0, "{}", many.stderr.tail);
    assert_eq!(media(&many).len(), 32);
    assert!(
        many.stdout
            .delta
            .ends_with("[medium 32: image/png, 16 bytes]\n[medium not kept: a job keeps at most 32 media and 64 MiB]\n"),
        "{}",
        many.stdout.delta
    );

    // Four media of 16 MiB fill the job's 64 MiB: the fifth is not kept, and
    // the command goes on.
    let large = run(&shell, "demi medium --count 5 --size 16777216 --after done").await;
    assert_eq!(exited(&large), 0, "{}", large.stderr.tail);
    let line = |number| format!("[medium {number}: image/png, 16777216 bytes]\n");
    assert_eq!(
        large.stdout.delta,
        format!(
            "{}{}{}{}[medium not kept: a job keeps at most 32 media and 64 MiB]\ndone",
            line(1),
            line(2),
            line(3),
            line(4)
        )
    );
    let sizes: Vec<_> = media(&large)
        .into_iter()
        .map(|(number, _, bytes)| (number, bytes.len()))
        .collect();
    assert_eq!(sizes, [1, 2, 3, 4].map(|number| (number, 16 * 1024 * 1024)));

    // A leaf that does not declare media, and bytes that are no medium,
    // fail their command; nothing is kept.
    let refused = run(
        &shell,
        "demi undeclared; echo \"undeclared $?\"; demi medium --kind text; echo \"text $?\"",
    )
    .await;
    assert_eq!(refused.stdout.delta, "undeclared 1\ntext 1\n");
    assert!(
        refused.stderr.delta.contains("demi undeclared: returned a medium, but its declaration does not say it returns media")
            && refused.stderr.delta.contains("demi medium: returned a medium that is no image or video a model reads"),
        "{}",
        refused.stderr.delta
    );
    assert_eq!(media(&refused), []);
    fixture.stop().await;
}
