//! A command's whole output (`runtime.md` § Results and previews, § The
//! whole output): the backend stores it when the command ends, and its Host
//! then keeps nothing of it; the result that reports the end shows its first
//! and last lines and names the rest; `demi shell output` prints what the
//! result names a page at a time, the newest lines, or the bytes as they are,
//! and what a running command's Host kept so far. The model is an Anthropic
//! endpoint the test scripts; the device is a real runner.

use demi_agent_tools::testing::{field, shown_output};
use demi_provider_common::testing::MockVendor;
use demi_shared_types::Block;
use demi_web_api_protocol::conversations::CommandCall;
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;

use crate::conversations::{anthropic_at, create, on_device, transcript};
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
        page[0].ends_with(" of 30000, stdout and stderr, exit code 0]"),
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
            "[command {command}: lines 29999-30000 of 30000, stdout and stderr, exit code 0]\n 29999\t29999\n 30000\t30000\n\
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

// About 1.5 s: a real device pairs, and the first call waits out its window
// of a second.
//
// A command whose script has ended but whose background task runs on says
// so and names the task, in its result and in `demi shell status`, and its
// stop stops the task. Before, the result said only that the command keeps
// running, which a model took for a hang.
#[tokio::test]
async fn a_command_whose_script_ended_names_the_background_task_that_keeps_it_running() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let (_device, _) = on_device(&harness, &backend, &master, CONVERSATION).await;
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;
    let line = "the script has ended; its background task \"sleep 30\" keeps the command running, and stopping the command stops it";

    let started = work
        .turn(vec![shell("t1", "sleep 30 & echo started", 1_000), say("running")])
        .await;
    let result = &started.received[0];
    assert!(result.starts_with("status: running"), "{result}");
    assert!(result.contains(&format!("output:\nstarted\n{line}\nnext: ")), "{result}");
    let command = field(result, "commandId").to_owned();

    let look = format!("demi shell status {command}");
    let looked = work
        .turn(vec![shell("t2", &look, 30_000), say("looked")])
        .await;
    let shown = shown_output(&looked.received[0]);
    assert!(shown.starts_with("status: running"), "{shown}");
    assert!(shown.contains(&format!("\n{line}\n")), "{shown}");

    let stop = format!("demi shell stop {command}");
    let stopped = work
        .turn(vec![shell("t3", &stop, 30_000), say("stopped")])
        .await;
    assert_eq!(
        shown_output(&stopped.received[0]),
        format!("[command {command} stopped]\n")
    );
    backend.close().await;
}

// A few seconds: a real device installs the builtin package, and one turn
// runs a shell job.
//
// Planted defects this catches: a viewed medium that does not reach the
// result over a real runner; a PDF that the Anthropic API's request leaves
// out of the tool result or carries as anything but a `document` block; and
// a PDF from stdin without the name its number gives it.
#[tokio::test]
async fn viewed_media_are_attached_to_the_result_and_a_pdf_rides_as_a_document_block() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let (_device, root) = on_device(&harness, &backend, &master, CONVERSATION).await;
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;
    let png = demi_agent_store::testing::png(4, 3, 1).into_bytes();
    std::fs::write(format!("{root}/shot.png"), &png).unwrap();
    let pdf = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n%%EOF\n";
    std::fs::write(format!("{root}/report.pdf"), pdf).unwrap();

    let viewed = work
        .turn(vec![
            shell("t1", "demi file view shot.png > /dev/null && cat report.pdf | demi file view", 30_000),
            say("viewed"),
        ])
        .await;
    let result = &viewed.received[0];
    assert_eq!(
        shown_output(result),
        format!(
            "[image 1: image/png, 4 × 3 px, {} bytes]\n[document 2: document-2.pdf, application/pdf, {} bytes]\n[image]\n[document]\n",
            png.len(),
            pdf.len()
        ),
        "{result}"
    );
    let document = viewed
        .requests
        .last()
        .and_then(|request| {
            request["messages"]
                .as_array()?
                .iter()
                .flat_map(|message| message["content"].as_array().into_iter().flatten())
                .filter(|block| block["type"] == "tool_result")
                .flat_map(|block| block["content"].as_array().into_iter().flatten())
                .find(|part| part["type"] == "document")
                .cloned()
        })
        .expect("the tool result carries a document");
    assert_eq!(document["source"]["media_type"], "application/pdf");
    assert_eq!(document["title"], "document-2.pdf");
    backend.close().await;
}

// A few seconds: a real device installs the builtin package, and two turns
// run a shell job each.
#[tokio::test]
async fn a_commands_record_keeps_how_it_ended_for_its_page_header() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, ENDED).await;
    let _device = on_device(&harness, &backend, &master, ENDED).await;
    let mut work = Driven::open(&backend, &master, &vendor, ENDED, &provider, "/work").await;

    // The command ends within its call, which gives its end and lets its
    // handle go: from then on only its record knows how it ended.
    let failed = work
        .turn(vec![shell("t1", "echo failing; exit 3", 30_000), say("failed")])
        .await;
    let command = field(&failed.received[0], "commandId").to_owned();
    assert_eq!(field(&failed.received[0], "exitCode"), "3");

    // A report's title brings the call that started it into view, wherever
    // the transcript is (`web-api.md` § Subagents and commands).
    let call = backend
        .get(&format!("/api/conversations/{ENDED}/commands/{command}"), Some(&master))
        .await;
    assert_eq!(call.status, StatusCode::OK, "{}", String::from_utf8_lossy(&call.body));
    let call: CommandCall = call.json();
    let started = transcript(&backend, &master, ENDED)
        .await
        .blocks
        .into_iter()
        .find(|block| matches!(block, Block::ToolCall(_)))
        .expect("the shell call");
    assert_eq!((&call.block_id, call.subagent_id), (started.id(), None));
    let unknown = backend
        .get(&format!("/api/conversations/{ENDED}/commands/999"), Some(&master))
        .await;
    assert_eq!(unknown.refusal(), (StatusCode::NOT_FOUND, ErrorCode::NotFound));

    // `demi shell output`'s header names the end its record keeps.
    let read = work
        .turn(vec![
            shell("t2", &format!("demi shell output {command}"), 30_000),
            say("read"),
        ])
        .await;
    assert_eq!(
        shown_output(&read.received[0]),
        format!("[command {command}: lines 1-1 of 1, stdout and stderr, exit code 3]\n     1\tfailing\n")
    );

    backend.close().await;
}

const ENDED: &str = "5e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a06";
