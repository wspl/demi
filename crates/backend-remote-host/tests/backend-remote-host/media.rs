//! The media a job's commands view (`runtime.md` § Media the model views,
//! `commands.md` § Return media) over a real runner: each medium goes to
//! the job whatever the command's stdout is, its line goes into the job's
//! output as it arrives, and the backend has each one the job kept when the
//! job ended.

use demi_backend_remote_host::{
    RemoteShellEnvironment,
    testing::{FixtureOptions, NativeFixture, RunnerFixture},
};
use demi_host_interface::{CommandSet, CommandState, CommandStatus, GroupBuilder};
use schemars::JsonSchema;

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

/// A runner whose jobs run `demi medium`, the fixture's media, and `demi
/// undeclared`, the same operation under a leaf that does not declare
/// media.
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
                .leaf(native_leaf(&native, "undeclared", "medium").input::<MediumArgs>()),
        )
        .unwrap();
    let selection = catalog(&native).select(&commands).unwrap();
    let fixture = RunnerFixture::start(FixtureOptions {
        commands,
        ..FixtureOptions::default()
    })
    .await;
    let shell = shell_on(fixture.host(), &[], Some(selection));
    (fixture, shell)
}

/// The media an ended command's job kept: each one's number, type and
/// bytes.
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

fn png_medium(number: u32) -> (u32, String, Vec<u8>) {
    (number, "image/png".to_owned(), png(1))
}

fn line(number: u32) -> String {
    format!("[image {number}: image/png, 16 bytes]\n")
}

/// About 1.5 s here: two jobs, each a login shell.
///
/// Planted defects this catches: a medium written into a stdout that is not
/// the job's output (the file and the pipe would receive its bytes, and the
/// substitution would hold them); its line written into the command's stdout
/// rather than the job's output (the pipe would count it, and the line would
/// be missing); an alias that does not hand its media to the job; and media
/// numbered other than in the order they reached the runner.
#[tokio::test(flavor = "local")]
async fn a_medium_goes_to_the_job_whatever_the_commands_stdout_is() {
    let (fixture, shell) = media_runner().await;
    let home = fixture.home().to_owned();
    let script = [
        "echo before",
        "demi medium > shot.png",
        "demi medium | wc -c | tr -d ' '",
        "x=$(demi medium); echo \"[$x]\"",
        "for i in 1 2; do demi medium; done",
        "echo --count=1 | xargs demi medium",
        "echo after",
    ]
    .join("\n");
    let status = run(&shell, &script).await;
    assert_eq!(exited(&status), 0, "{}", status.stderr.tail);
    assert_eq!(
        status.stdout.delta,
        format!(
            "before\n{}{}0\n{}[]\n{}{}{}after\n",
            line(1),
            line(2),
            line(3),
            line(4),
            line(5),
            line(6),
        )
    );
    assert_eq!(std::fs::read(format!("{home}/shot.png")).unwrap(), b"");
    assert_eq!(media(&status), (1..=6).map(png_medium).collect::<Vec<_>>());

    // A medium after output that ends no line stands on a line of its own.
    let unended = run(&shell, "printf partial; demi medium").await;
    assert_eq!(unended.stdout.delta, format!("partial\n{}", line(1)));
    fixture.stop().await;
}

/// About 1 s here: three jobs, one of them viewing 80 MiB of media.
///
/// Planted defects this catches: a medium the runner keeps past its count
/// or its bytes, a medium beyond the bounds that stops its command, and a
/// check that lets a leaf without `media`, or bytes that are no image, video
/// or PDF, return one.
#[tokio::test(flavor = "local")]
async fn media_keep_their_bounds_and_their_checks() {
    let (fixture, shell) = media_runner().await;

    // The 33rd medium is not kept: its line says so, and the command goes
    // on and succeeds.
    let many = run(&shell, "demi medium --count 33").await;
    assert_eq!(exited(&many), 0, "{}", many.stderr.tail);
    assert_eq!(media(&many).len(), 32);
    assert!(
        many.stdout.delta.ends_with(&format!(
            "{}[image 33: not kept: a job keeps at most 32 media and 64 MiB]\n",
            line(32)
        )),
        "{}",
        many.stdout.delta
    );

    // Four media of 16 MiB fill the job's 64 MiB: the fifth is not kept, and
    // the command goes on.
    let large = run(&shell, "demi medium --count 5 --size 16777216; echo done").await;
    assert_eq!(exited(&large), 0, "{}", large.stderr.tail);
    let big = |number| format!("[image {number}: image/png, 16777216 bytes]\n");
    assert_eq!(
        large.stdout.delta,
        format!(
            "{}{}{}{}[image 5: not kept: a job keeps at most 32 media and 64 MiB]\ndone\n",
            big(1),
            big(2),
            big(3),
            big(4)
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
            && refused.stderr.delta.contains("demi medium: returned a medium that is no image, video or PDF a model reads"),
        "{}",
        refused.stderr.delta
    );
    assert_eq!(media(&refused), []);
    fixture.stop().await;
}
