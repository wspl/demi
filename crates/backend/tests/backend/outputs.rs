//! A command's whole output (`runtime.md` § Results and previews, § The
//! whole output): the backend stores it when the command ends, and its Host
//! then keeps nothing of it; the result that reports the end shows its first
//! and last lines and names the rest; `demi shell output` prints what the
//! result names a page at a time, the newest lines, or the bytes as they are,
//! and what a running command's Host kept so far. The model is an Anthropic
//! endpoint the test scripts; the device is a real runner.

use demi_agent_tools::testing::{field, shown_output};
use demi_provider_common::testing::MockVendor;

use crate::conversations::{anthropic_at, create, on_device};
use crate::support::{Harness, eventually};
use crate::work::{Driven, say, shell};

const CONVERSATION: &str = "5e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a05";

// Several seconds: a real device installs the builtin package, and six turns
// run a shell job each.
#[tokio::test]
async fn a_long_outputs_result_names_what_it_leaves_out_and_demi_shell_output_prints_it() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let (device, _) = on_device(&harness, &backend, &master, CONVERSATION).await;
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;

    // 30,000 lines, 168,894 bytes: beyond the 32 KiB of each stream the
    // backend receives while the command runs.
    let counted = work
        .turn(vec![shell("t1", "seq 1 30000", 30_000), say("counted")])
        .await;
    let result = &counted.received[0];
    assert!(
        result.chars().count() <= 16_000,
        "{} characters",
        result.chars().count()
    );
    let command = field(result, "commandId").to_owned();
    let output = shown_output(result);
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!((lines[0], *lines.last().unwrap()), ("1", "30000"));
    let between = lines
        .iter()
        .position(|line| line.starts_with("[... lines "))
        .expect("the line between the start and the end");
    let first = lines[between - 1].parse::<u64>().unwrap() + 1;
    let last = lines[between + 1].parse::<u64>().unwrap() - 1;
    let bytes: usize = (first..=last)
        .map(|number| number.to_string().len() + 1)
        .sum();
    let read = format!("demi shell output {command} --lines {first}-{last}");
    assert_eq!(
        lines[between],
        format!("[... lines {first}-{last} not shown ({bytes} bytes); read them: {read} ...]")
    );
    // The Host keeps nothing of the ended command.
    let jobs = device.runner.state_dir().join("jobs");
    eventually("the device keeps no job directory", || {
        let held = std::fs::read_dir(&jobs)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .any(|entry| entry.path().is_dir())
            })
            .unwrap_or(false);
        async move { !held }
    })
    .await;

    // The command the line names prints those lines, a page at a time,
    // numbered as `cat -n` numbers them.
    let paged = work
        .turn(vec![shell("t2", &read, 30_000), say("read")])
        .await;
    let page = shown_output(&paged.received[0]);
    let page: Vec<&str> = page.lines().collect();
    assert!(
        page[0].starts_with(&format!("[command {command}: lines {first}-")),
        "{}",
        page[0]
    );
    assert!(
        page[0].ends_with(" of 30000, stdout and stderr]"),
        "{}",
        page[0]
    );
    assert_eq!(page[1], format!("{first:>6}\t{first}"));
    let shown: u64 = page[page.len() - 2]
        .split('\t')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let next = format!(
        "[next: demi shell output {command} --lines {}-{last}]",
        shown + 1
    );
    assert_eq!(*page.last().unwrap(), next);
    assert!(
        page.iter()
            .map(|line| line.chars().count() + 1)
            .sum::<usize>()
            <= 12_000
    );

    // `grep -n` on the bytes gives the numbers pages take, and a reader that
    // stops early ends the call quietly.
    let script = format!(
        "demi shell output {command} --raw | grep -n '^12345$'; demi shell output {command} --raw | head -n 2"
    );
    let searched = work
        .turn(vec![shell("t3", &script, 30_000), say("searched")])
        .await;
    assert_eq!(shown_output(&searched.received[0]), "12345:12345\n1\n2\n");

    // The newest lines; a range past the end, and a command the
    // conversation does not have, fail.
    let script = format!(
        "demi shell output {command} --tail 2; demi shell output {command} --lines 30001-30002; demi shell output nothing-here"
    );
    let tailed = work
        .turn(vec![shell("t4", &script, 30_000), say("tailed")])
        .await;
    assert_eq!(
        shown_output(&tailed.received[0]),
        format!(
            "[command {command}: lines 29999-30000 of 30000, stdout and stderr]\n 29999\t29999\n 30000\t30000\n\
             demi shell output: lines 30001-30002 are past the end: the output has 30000 lines\n\
             demi shell output: no command nothing-here in this conversation\n"
        )
    );

    // A running command's output reads as its Host keeps it so far.
    let started = work
        .turn(vec![
            shell("t5", "seq 1 20000; sleep 30", 1_000),
            say("running"),
        ])
        .await;
    let result = &started.received[0];
    assert!(result.starts_with("status: running"), "{result}");
    let running = field(result, "commandId").to_owned();
    let script = format!("demi shell output {running} --tail 1");
    let newest = work
        .turn(vec![shell("t6", &script, 30_000), say("newest")])
        .await;
    assert_eq!(
        shown_output(&newest.received[0]),
        format!(
            "[command {running}: lines 20000-20000 of 20000 so far, stdout and stderr]\n 20000\t20000\n"
        )
    );
    backend.close().await;
}
