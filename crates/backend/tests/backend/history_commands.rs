//! An agent reads its own earlier history (`compaction.md` § Reading the
//! earlier history, § What the model receives afterward): after a
//! compaction the next request tells the model that `demi conversation
//! history` finds and reads what the summary left out; `read` prints a
//! range of the agent's transcript, messages and texts whole and other
//! blocks as one line each, up to about 8,000 characters; `open` prints a
//! command's script and its output a page at a time, a thinking block, a
//! subagent's report, or saves a message's attachment under the scratch
//! directory; a subagent reads its own transcript, where its brief is its
//! parent's. The model is the scripted family of the subagent scenarios,
//! the device a real runner. No test calls a real model.

use std::sync::Arc;
use std::time::Duration;

use jiff::SignedDuration;

use demi_backend_blobs::counting::ObjectCounts;
use demi_conversation_socket_protocol::ClientFrame;
use demi_provider_common::testing::event;
use demi_shared_types::{Block, ToolResultContentBlock};
use serde_json::json;

use crate::conversations::{FIRST, Socket, send, transcript};
use crate::subagents::{Answer, Scripts, say, shell, tree_on};
use crate::support::{Harness, Session, TestBackend, eventually};
use crate::uploads::{upload, with_upload};

/// The model's shell call `id` titled `title`, running `script`.
fn titled(id: &str, title: &str, script: &str) -> Answer {
    vec![
        event::tool_call(
            id,
            "shell",
            json!({ "description": title, "script": script, "intervalMs": 60_000 }),
        ),
        event::response(1, 1),
    ]
}

/// The text of the result of the root's call `id`, as the transcript keeps
/// it.
async fn result(backend: &TestBackend, session: &Session, id: &str) -> String {
    let blocks = transcript(backend, session, FIRST).await.blocks;
    blocks
        .iter()
        .find_map(|block| match block {
            Block::ToolCall(call) if call.tool_use_id == id => Some(
                call.output
                    .iter()
                    .filter_map(|part| match part {
                        ToolResultContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no call {id} in {blocks:?}"))
}

/// How many of `text`'s characters stand in runs of 100 or more of
/// `character`: what a test's long message or script shows of itself.
fn runs(text: &str, character: char) -> usize {
    let mut total = 0;
    let mut run = 0;
    for next in text.chars().chain(['\0']) {
        if next == character {
            run += 1;
            continue;
        }
        if run >= 100 {
            total += run;
        }
        run = 0;
    }
    total
}

/// The index of the first of the root's blocks `wanted` accepts.
async fn index_of(backend: &TestBackend, session: &Session, wanted: impl Fn(&Block) -> bool) -> usize {
    let blocks = transcript(backend, session, FIRST).await.blocks;
    blocks
        .iter()
        .position(wanted)
        .unwrap_or_else(|| panic!("no such block in {blocks:?}"))
}

// Several seconds: a real device runs the shell jobs of two turns, one of
// them a command of 3,000 lines, around a compaction.
#[tokio::test]
async fn an_agent_reads_and_opens_what_its_compaction_summarized() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _laptop, _root, _) = tree_on(&scripts, Harness::new()).await;
    let spec = upload(&backend, &master, "spec.md", "text/markdown", b"Name what sets Demi apart.").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The answer measures more than half the model's window of 100,000
    // tokens, from which the user may compact.
    scripts.root(
        FIRST,
        vec![
            {
                let mut call = vec![event::thinking("The list should stay short.")];
                call.extend(titled("t1", "Rewrite the README feature list", "seq 1 3000"));
                call
            },
            vec![event::text("Noted the README rules."), event::response(60_000, 1)],
            say("The user set the README's rules."),
        ],
    );
    socket
        .send(&with_upload(
            "m1",
            "The README's feature list should name what sets Demi apart",
            &[(&spec, "spec.md")],
        ))
        .await;
    socket.until_idle().await;
    socket.send(&ClientFrame::Compact { node: None }).await;
    socket.until_idle().await;

    let message = index_of(&backend, &master, |block| matches!(block, Block::User(_))).await;
    let thinking = index_of(&backend, &master, |block| matches!(block, Block::Thinking(_))).await;
    let command = index_of(&backend, &master, |block| matches!(block, Block::ToolCall(_))).await;
    let answer = index_of(&backend, &master, |block| matches!(block, Block::Text(_))).await;
    let summary = index_of(&backend, &master, |block| matches!(block, Block::CompactionBoundary(_))).await;
    scripts.root(
        FIRST,
        vec![
            shell("t2", &format!("demi conversation history read {message}..{answer}")),
            shell("t3", &format!("demi conversation history read '#{summary}..' && demi conversation history read ..{message}")),
            shell("t4", &format!("demi conversation history open {command}")),
            shell("t4b", &format!("demi conversation history open {command} --page 2")),
            // A job whose DEMI_SCRATCH names another directory saves into
            // the conversation's own all the same.
            shell(
                "t5",
                &format!(
                    "own=\"$DEMI_SCRATCH\" && DEMI_SCRATCH=\"$PWD/elsewhere\" demi conversation history open {message}/a1 && cat \"$own/{message}-a1-spec.md\" && test ! -e elsewhere && printf '\\nown: %s\\n' \"$own\""
                ),
            ),
            shell("t6", &format!("demi conversation history open {thinking} && demi conversation history open {summary}")),
            say("Rewritten."),
        ],
    );
    socket.chat("m2", "Rewrite it by those rules").await;

    // The request after the pass tells the model where the rest is.
    let asked = scripts.asked(|session| session == FIRST);
    let after = &asked[asked.len() - 7];
    assert!(
        after.contains("What the summary leaves out is still in the conversation: `demi conversation history` finds and reads it."),
        "{after}"
    );

    // The message and the answer whole, the attachment and the command as
    // a line each, and no thinking.
    let read = result(&backend, &master, "t2").await;
    assert!(read.contains(&format!("[{message} user · ")), "{read}");
    assert!(read.contains("The README's feature list should name what sets Demi apart"), "{read}");
    assert!(read.contains(&format!("[{message}/a1 attachment: spec.md]")), "{read}");
    assert!(read.contains(&format!("[{command} command: Rewrite the README feature list · exit 0 · ")), "{read}");
    assert!(read.contains(&format!("[{answer} agent · ")) && read.contains("Noted the README rules."), "{read}");
    assert!(!read.contains("The list should stay short."), "{read}");
    // An open end reads to the transcript's end, the summary as a line; an
    // open start from the first block.
    let ends = result(&backend, &master, "t3").await;
    assert!(ends.contains(&format!("[{summary} summary of the earlier conversation: demi conversation history open {summary}]")), "{ends}");
    assert!(ends.contains("Rewrite it by those rules"), "{ends}");

    // A command: its title, its script, and its output a page at a time,
    // as demi shell output pages it.
    let opened = result(&backend, &master, "t4").await;
    assert!(opened.contains(&format!("output:\n[{command} command: Rewrite the README feature list · exit 0 · ")), "{opened}");
    assert!(opened.contains("\n$ seq 1 3000\n[command "), "{opened}");
    assert!(opened.contains(": lines 1-"), "{opened}");
    assert!(opened.contains(" of 3000, stdout and stderr, exit code 0]\n     1\t1\n"), "{opened}");
    // Its next page is named within `history`, never by `demi shell
    // output`, and starts where this one ends.
    assert!(!opened.contains("demi shell output"), "{opened}");
    let shown_to: u64 = opened
        .split(": lines 1-")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|last| last.parse().ok())
        .unwrap_or_else(|| panic!("{opened}"));
    let pages = opened
        .split("Page 1 of ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or_else(|| panic!("{opened}"));
    assert!(
        opened.ends_with(&format!("Page 1 of {pages} · next: demi conversation history open {command} --page 2")),
        "{opened}"
    );
    let second = result(&backend, &master, "t4b").await;
    let from = shown_to + 1;
    assert!(second.contains(&format!(": lines {from}-")), "{second}");
    assert!(second.contains(&format!("{from:>6}\t{from}\n")), "{second}");

    // The attachment is saved under the scratch directory, where the
    // ordinary tools read it.
    let saved = result(&backend, &master, "t5").await;
    assert!(saved.contains(&format!("[{message}/a1 attachment: spec.md] saved to ")), "{saved}");
    assert!(saved.contains("Name what sets Demi apart."), "{saved}");
    let own = saved.lines().find_map(|line| line.strip_prefix("own: ")).unwrap_or_else(|| panic!("{saved}"));
    assert!(saved.contains(&format!("saved to {own}/{message}-a1-spec.md")), "{saved}");

    let thought = result(&backend, &master, "t6").await;
    assert!(thought.contains(&format!("[{thinking} thinking]\nThe list should stay short.")), "{thought}");
    assert!(thought.contains("The user set the README's rules."), "{thought}");
    backend.close().await;
}

// A few seconds: eight turns of a scripted model, and a real device runs the
// shell job of the last.
#[tokio::test]
async fn a_read_stops_at_about_eight_thousand_characters_and_cuts_a_longer_message() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _laptop, _root, _) = tree_on(&scripts, Harness::new()).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    for (turn, letter) in ["a", "b", "c"].iter().enumerate() {
        scripts.root(FIRST, vec![say(&letter.repeat(3_000))]);
        socket.chat(&format!("m{turn}"), &format!("Answer {letter}")).await;
    }
    let first = index_of(&backend, &master, |block| matches!(block, Block::User(_))).await;
    let long = "d".repeat(9_000);
    scripts.root(
        FIRST,
        vec![
            shell("t1", &format!("demi conversation history read {first}..")),
            say("read"),
        ],
    );
    socket.chat("m3", &long).await;
    let message = index_of(&backend, &master, |block| matches!(block, Block::User(user) if user.turn_id.as_str() == "m3")).await;
    scripts.root(
        FIRST,
        vec![
            shell("t2", &format!("demi conversation history read {message}..{}", message + 1)),
            say("read"),
        ],
    );
    socket.chat("m4", "Again").await;
    // A message that fits a read's budget alone, but not with the line that
    // names it: the page the read's cut line names exists, and runs as the
    // line prints it.
    scripts.root(FIRST, vec![say("noted")]);
    socket.chat("m6", &"e".repeat(7_990)).await;
    let near = index_of(&backend, &master, |block| matches!(block, Block::User(user) if user.turn_id.as_str() == "m6")).await;
    // A command whose script is longer than a page: `open` pages the script
    // and its output together, each page within a shell result's page.
    let script = format!(": {} && seq 1 2000", "s".repeat(15_000));
    scripts.root(
        FIRST,
        vec![
            titled("t4", "Run the long script", &script),
            shell(
                "t5",
                &format!(
                    "line=$(demi conversation history read {near}..{near} | tail -n 1) && rest=${{line#*rest: }} && eval \"$rest\""
                ),
            ),
            say("ran"),
        ],
    );
    socket.chat("m7", "Run it").await;
    let command = index_of(&backend, &master, |block| matches!(block, Block::ToolCall(call) if call.tool_use_id == "t4")).await;
    scripts.root(
        FIRST,
        vec![
            shell("t6", &format!("demi conversation history open {command}")),
            shell("t7", &format!("demi conversation history open {command} --page 2")),
            shell("t8", &format!("demi conversation history open {command} --page 3")),
            shell("t9", &format!("demi conversation history open {command} --page 4")),
            say("opened"),
        ],
    );
    socket.chat("m8", "Open it").await;
    // It is another year now: the blocks of the last one show theirs. The
    // clock moves 20 days at a time, and a request renews the session each
    // time, which would expire 30 days after its last renewal.
    for _ in 0..5 {
        harness.clock.advance(SignedDuration::from_hours(24 * 20));
        transcript(&backend, &master, FIRST).await;
    }
    scripts.root(
        FIRST,
        vec![
            shell("t3", &format!("demi conversation history open {message} --page 2")),
            say("opened"),
        ],
    );
    socket.chat("m5", "And the rest").await;

    // Two answers of 3,000 fit, and the third would pass 8,000: the read
    // stops at the block before it and names the read that goes on.
    let read = result(&backend, &master, "t1").await;
    assert!(read.contains(&"b".repeat(3_000)), "{read}");
    assert!(!read.contains(&"c".repeat(3_000)), "{read}");
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let third = blocks
        .iter()
        .position(|block| matches!(block, Block::Text(text) if text.text.starts_with('c')))
        .unwrap();
    assert!(
        read.ends_with(&format!("Stopped at {} · next: demi conversation history read {third}..", third - 1)),
        "{read}"
    );
    // A message longer than a read is cut, and the line names where its
    // rest is and where the read goes on.
    let cut = result(&backend, &master, "t2").await;
    assert!(!cut.contains(&long), "{cut}");
    assert!(
        cut.ends_with(&format!(
            "Cut {message} · rest: demi conversation history open {message} --page 2 · next: demi conversation history read {}..{}",
            message + 1,
            message + 1
        )),
        "{cut}"
    );
    // The read shows the first page `open` shows, so the two pages hold the
    // message once.
    let shown = |text: &str| runs(text, 'd');
    let first_page = shown(&cut);
    let near_rest = result(&backend, &master, "t5").await;
    assert!(near_rest.contains(&format!("[{near} user · ")), "{near_rest}");
    assert!(near_rest.contains("eeeeeeeeee"), "{near_rest}");
    assert!(!near_rest.contains("has 1 pages"), "{near_rest}");
    // Each page of the command holds the head and stays within a page;
    // together they hold the script and the output once each.
    let pages: Vec<String> = [
        result(&backend, &master, "t6").await,
        result(&backend, &master, "t7").await,
        result(&backend, &master, "t8").await,
        result(&backend, &master, "t9").await,
    ]
    .into();
    let mut scripted = 0;
    for page in &pages {
        let printed = page.split_once("output:\n").map_or(page.as_str(), |(_, printed)| printed);
        assert!(printed.chars().count() <= 12_000, "{} characters: {printed}", printed.chars().count());
        assert!(printed.starts_with(&format!("[{command} command: Run the long script · exit 0 · ")), "{printed}");
        scripted += runs(printed, 's');
    }
    assert!(pages[0].contains("Page 1 of 4 · next: demi conversation history open"), "{}", pages[0]);
    assert!(!pages[0].contains("[command "), "the script fills the first page: {}", pages[0]);
    assert!(pages[1].contains("[command ") && pages[1].contains("     1\t1\n"), "{}", pages[1]);
    assert!(pages[3].contains("  2000\t2000"), "{}", pages[3]);
    assert!(!pages[3].contains("Page "), "{}", pages[3]);
    assert_eq!(scripted, 15_000);
    // A time of this year has no year.
    assert!(read.contains(&format!("[{first} user · Sep 24, 8:00 AM]")), "{read}");
    // The rest of the message, its last page, which names no next one, and
    // its time of another year with the year.
    let rest = result(&backend, &master, "t3").await;
    assert!(rest.contains(&format!("[{message} user · Sep 24, 2026, 8:00 AM]\nddd")), "{rest}");
    assert_eq!(first_page + shown(&rest), 9_000, "{cut}\n{rest}");
    assert!(!rest.contains("Page "), "{rest}");
    backend.close().await;
}

// A few seconds: a real device runs the shell jobs of a parent and its
// child.
#[tokio::test]
async fn a_subagent_reads_its_own_transcript_and_its_parent_opens_its_report() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _laptop, _root, _) = tree_on(&scripts, Harness::new()).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.child(vec![
        shell("c1", "demi conversation history read 0.."),
        titled("c2", "Look for the release switch", "true"),
        shell("c3", "demi conversation history search release switch"),
        say("The flag is --release."),
    ]);
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi agent spawn --description finder <<< 'Find the build flag, the release switch'"),
            say("dispatched"),
            // The child's report wakes the idle parent.
            shell(
                "t2",
                "n=$(demi conversation history read 0.. | sed -n 's/^\\[\\([0-9]*\\) report.*/\\1/p') && demi conversation history open \"$n\"",
            ),
            say("received"),
        ],
    );
    socket.chat("m1", "Find the release switch for me").await;
    socket.until_idle().await;

    // The child's brief is its parent's message, in its own transcript.
    let children = scripts.asked(|session| session != FIRST);
    let child = children.last().unwrap();
    assert!(child.contains(" parent · "), "{child}");
    assert!(child.contains("Find the build flag"), "{child}");
    assert!(!child.contains(" user · "), "{child}");
    // Its search finds its own brief and command, not the root's rows.
    assert!(child.contains("] Find the build flag, the **release** **switch**"), "{child}");
    assert!(child.contains(" command] Look for the **release** **switch** · exit 0 · "), "{child}");
    assert!(child.contains("Page 1 of 1"), "{child}");
    assert!(!child.contains("switch** for me"), "the root's message is the root's: {child}");
    // Nor does the product's search show a subagent's rows, which the
    // index holds since the child's search found them.
    let product = backend.get("/api/search?q=Look", Some(&master)).await;
    assert!(String::from_utf8_lossy(&product.body).contains(r#""results":[]"#), "{}", String::from_utf8_lossy(&product.body));
    // The parent reads the report as a line and opens it.
    let report = result(&backend, &master, "t2").await;
    assert!(report.contains(" report of agent 1 (finder) · completed]"), "{report}");
    assert!(report.contains("The flag is --release."), "{report}");
    backend.close().await;
}

// About a second: ten turns of a scripted model, the index following each
// within the 20 ms interval the test sets, and a real device runs the shell
// jobs of the last.
#[tokio::test]
async fn an_agent_searches_its_history_for_every_word_or_any_and_pages_the_hits() {
    let scripts = Arc::new(Scripts::default());
    let mut harness = Harness::new();
    harness.conversations.search_interval = Duration::from_millis(20);
    let (_harness, backend, master, _laptop, _root, _) = tree_on(&scripts, harness).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let mut said = Vec::new();
    for rule in 0..6 {
        said.push(format!("The README feature list keeps rule {rule}"));
    }
    said.push("Only the README here".to_owned());
    for build in 0..6 {
        said.push(format!("Build {build} with --release"));
    }
    said.push("模块路径不对".to_owned());
    for (turn, text) in said.iter().enumerate() {
        scripts.root(FIRST, vec![say(&format!("Answer {turn}"))]);
        socket.chat(&format!("m{turn}"), text).await;
    }
    scripts.root(FIRST, vec![titled("t0", "Deploy the staging server", "true"), say("Deployed")]);
    socket.chat("ship", "Ship it").await;
    eventually("the index holds the last answer", || async {
        let found = backend.get("/api/search?q=Deployed", Some(&master)).await;
        String::from_utf8_lossy(&found.body).contains("Deployed")
    })
    .await;
    // The product's search finds the root's messages and answers alone: a
    // command's title is not among them.
    let product = backend.get("/api/search?q=staging", Some(&master)).await;
    assert!(String::from_utf8_lossy(&product.body).contains(r#""results":[]"#), "{}", String::from_utf8_lossy(&product.body));
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let at = |wanted: &str| {
        blocks
            .iter()
            .position(|block| match block {
                Block::User(user) => format!("{:?}", user.content).contains(wanted),
                Block::Text(text) => text.text == wanted,
                _ => false,
            })
            .unwrap_or_else(|| panic!("no {wanted} in {blocks:?}"))
    };
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi conversation history search README feature list"),
            shell("t2", "demi conversation history search README feature list --page 2"),
            shell("t3", "demi conversation history search readme LIST --any"),
            shell("t4", "demi conversation history search 路径"),
            shell("t5", "demi conversation history search nowhere"),
            shell("t6", "demi conversation history search staging"),
            // The continuation runs as it is printed, a word that looks
            // like an option included.
            shell(
                "t7",
                "line=$(demi conversation history search -- --release | tail -n 1) && eval \"${line#*next: }\"",
            ),
            say("found"),
        ],
    );
    socket.chat("find", "Find what I said").await;

    // Every word, the newest first, five to a page: each hit's index, who
    // and when, its text with the words marked, and the message before and
    // after it.
    let first = result(&backend, &master, "t1").await;
    let newest = at("rule 5");
    assert!(first.contains(&format!("\n{newest} · user · ")), "{first}");
    assert!(first.contains(&format!("  [{newest} user] The **README** **feature** **list** keeps rule 5\n")), "{first}");
    assert!(first.contains(&format!("  [{} agent] Answer 4\n", at("Answer 4"))), "{first}");
    assert!(first.contains(&format!("  [{} agent] Answer 5\n", at("Answer 5"))), "{first}");
    assert!(!first.contains("rule 0") && !first.contains("Only the README"), "{first}");
    assert!(
        first.contains("Page 1 of 2 · next: demi conversation history search --page 2 -- README feature list"),
        "{first}"
    );
    let second = result(&backend, &master, "t2").await;
    assert!(second.contains(&format!("  [{} user] The **README** **feature** **list** keeps rule 0", at("rule 0"))), "{second}");
    assert!(second.ends_with("Page 2 of 2"), "{second}");
    assert!(!second.contains("rule 1 "), "{second}");
    // Any word, case aside: the message with only one of them too.
    let any = result(&backend, &master, "t3").await;
    assert!(any.contains("  [") && any.contains("] Only the **README** here"), "{any}");
    assert!(any.contains("Page 1 of 2 · next: demi conversation history search --any --page 2 -- readme LIST"), "{any}");
    // A word of two Chinese characters, inside a longer text.
    let chinese = result(&backend, &master, "t4").await;
    assert!(chinese.contains(&format!("  [{} user] 模块**路径**不对", at("模块路径不对"))), "{chinese}");
    assert!(chinese.ends_with("Page 1 of 1"), "{chinese}");
    let none = result(&backend, &master, "t5").await;
    assert!(none.contains("Nothing of this transcript holds every one of: nowhere"), "{none}");
    let release = result(&backend, &master, "t7").await;
    assert!(release.contains("] Build 0 with **--release**"), "{release}");
    assert!(release.ends_with("Page 2 of 2"), "{release}");
    // A command by its title, with how it ended.
    let command = result(&backend, &master, "t6").await;
    let call = blocks
        .iter()
        .position(|block| matches!(block, Block::ToolCall(call) if call.tool_use_id == "t0"))
        .unwrap();
    assert!(command.contains(&format!("\n{call} · command · ")), "{command}");
    assert!(command.contains(&format!("  [{call} command] Deploy the **staging** server · exit 0 · ")), "{command}");
    backend.close().await;
}

// About a second: nine turns of a scripted model, the index following each
// within the 20 ms interval the test sets, and a real device runs the shell
// jobs of the last. The user is in Shanghai, eight hours ahead of UTC, and
// the second turn comes ten hours after the first: the next day there, the
// same day in UTC.
#[tokio::test]
async fn an_agent_narrows_its_history_search_by_who_wrote_it_and_when() {
    let scripts = Arc::new(Scripts::default());
    let mut harness = Harness::new();
    harness.conversations.search_interval = Duration::from_millis(20);
    let (harness, backend, master, _laptop, _root, _) = tree_on(&scripts, harness).await;
    let preferences = backend
        .patch(
            "/api/settings/preferences",
            &master,
            json!({ "locale": { "timeZone": "Asia/Shanghai", "languages": ["en"] }, "colorScheme": "light" }),
        )
        .await;
    assert!(preferences.status.is_success(), "{}", String::from_utf8_lossy(&preferences.body));
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    // September 24, 4:00 PM in Shanghai.
    scripts.root(FIRST, vec![say("The README intro is drafted")]);
    socket.chat("m0", "Draft the README intro").await;
    // September 25, 2:00 AM in Shanghai, still September 24 in UTC.
    harness.clock.advance(SignedDuration::from_hours(10));
    scripts.root(
        FIRST,
        vec![titled("t0", "Rewrite the README intro", "true"), say("The README intro is shorter")],
    );
    socket.chat("m1", "Shorten the README intro").await;
    for note in 0..5 {
        scripts.root(FIRST, vec![say(&format!("Noted {note}"))]);
        socket.chat(&format!("n{note}"), &format!("README note {note}")).await;
    }
    eventually("the index holds the last answer", || async {
        let found = backend.get("/api/search?q=Noted%204", Some(&master)).await;
        String::from_utf8_lossy(&found.body).contains("Noted 4")
    })
    .await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let at = |wanted: &str| {
        blocks
            .iter()
            .position(|block| match block {
                Block::User(user) => format!("{:?}", user.content).contains(wanted),
                Block::Text(text) => text.text == wanted,
                Block::ToolCall(call) => call.tool_use_id == wanted,
                _ => false,
            })
            .unwrap_or_else(|| panic!("no {wanted} in {blocks:?}"))
    };
    let (draft, drafted, shorten, rewrite, shorter) = (
        at("Draft the README"),
        at("The README intro is drafted"),
        at("Shorten the README"),
        at("t0"),
        at("The README intro is shorter"),
    );
    let notes: Vec<usize> = (0..5).map(|note| at(&format!("README note {note}"))).collect();
    let users = |indices: &[usize]| indices.iter().map(|at| format!("{at} · user")).collect::<Vec<_>>();
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi conversation history search --from user README"),
            shell("t2", "demi conversation history search --from agent README"),
            shell("t3", "demi conversation history search --from user --oldest README"),
            // The continuation of a filtered search runs as it is printed.
            shell(
                "t4",
                "line=$(demi conversation history search --from user --oldest README | tail -n 1) && echo \"$line\" && eval \"${line#*next: }\"",
            ),
            shell("t5", &format!("demi conversation history search --after {drafted} --before {} README", notes[0])),
            shell("t6", "demi conversation history search --before 2026-09-25 README"),
            shell(
                "t7",
                "demi conversation history search --from user --after 2026-09-24 --before 2026-09-26 --oldest README",
            ),
            shell("t8", &format!("demi conversation history read --from user {draft}..{shorter}")),
            shell("t9", &format!("demi conversation history read --from agent {draft}..{shorter}")),
            say("found"),
        ],
    );
    socket.chat("find", "Find what I asked").await;

    let heads = |text: &str| -> Vec<String> {
        text.lines()
            .filter(|line| line.split(" · ").next().is_some_and(|first| first.parse::<usize>().is_ok()))
            .map(|line| line.split(" · ").take(2).collect::<Vec<_>>().join(" · "))
            .collect()
    };
    // The user's messages alone, the newest first: no answer and no
    // command, and the next page names the filter.
    let user = result(&backend, &master, "t1").await;
    let newest: Vec<usize> = notes.iter().rev().copied().collect();
    assert_eq!(heads(&user), users(&newest), "{user}");
    assert!(user.contains("Page 1 of 2 · next: demi conversation history search --from user --page 2 -- README"), "{user}");
    // The agent's texts and commands alone.
    let agent = result(&backend, &master, "t2").await;
    assert_eq!(
        heads(&agent),
        vec![format!("{shorter} · agent"), format!("{rewrite} · command"), format!("{drafted} · agent")],
        "{agent}"
    );
    // The oldest first, and its second page as the first page's line runs.
    let oldest = result(&backend, &master, "t3").await;
    assert_eq!(heads(&oldest), users(&[draft, shorten, notes[0], notes[1], notes[2]]), "{oldest}");
    let second = result(&backend, &master, "t4").await;
    assert!(
        second.contains("Page 1 of 2 · next: demi conversation history search --from user --oldest --page 2 -- README\n"),
        "{second}"
    );
    assert_eq!(heads(&second), users(&notes[3..]), "{second}");
    assert!(second.ends_with("Page 2 of 2"), "{second}");
    // Between two blocks, neither included.
    let between = result(&backend, &master, "t5").await;
    assert_eq!(
        heads(&between),
        vec![format!("{shorter} · agent"), format!("{rewrite} · command"), format!("{shorten} · user")],
        "{between}"
    );
    // Before September 25 in Shanghai: the first turn alone, though the
    // second was on September 24 in UTC.
    let before = result(&backend, &master, "t6").await;
    assert_eq!(heads(&before), vec![format!("{drafted} · agent"), format!("{draft} · user")], "{before}");
    // After September 24 and before September 26 there: the second turn
    // on.
    let after = result(&backend, &master, "t7").await;
    assert_eq!(heads(&after), users(&[shorten, notes[0], notes[1], notes[2], notes[3]]), "{after}");
    assert!(
        after.contains(
            "next: demi conversation history search --from user --after 2026-09-24 --before 2026-09-26 --oldest --page 2 -- README"
        ),
        "{after}"
    );
    // A read of one side prints its messages alone, each with its index.
    let read = result(&backend, &master, "t8").await;
    assert!(read.contains(&format!("[{draft} user · Sep 24, 4:00 PM]\nDraft the README intro\n")), "{read}");
    assert!(read.contains(&format!("[{shorten} user · Sep 25, 2:00 AM]\nShorten the README intro")), "{read}");
    assert!(!read.contains(" agent · ") && !read.contains(" command: "), "{read}");
    // The agent's side is its texts and its command lines, as a search
    // counts them.
    let read = result(&backend, &master, "t9").await;
    assert!(
        read.contains(&format!("[{drafted} agent · Sep 24, 4:00 PM]\nThe README intro is drafted\n[{rewrite} command: Rewrite the README intro · exit 0 · ")),
        "{read}"
    );
    assert!(read.contains(&format!("[{shorter} agent · Sep 25, 2:00 AM]\nThe README intro is shorter")), "{read}");
    assert!(!read.contains(" user · "), "{read}");
    backend.close().await;
}

// A few seconds: a real device runs the shell jobs of three turns, one of
// them stopped while its call waits for the file's bytes.
#[tokio::test]
async fn an_open_stopped_while_it_reads_a_file_saves_nothing() {
    let counts = ObjectCounts::default();
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _laptop, _root, _) =
        tree_on(&scripts, Harness::new().with_object_counts(&counts)).await;
    let spec = upload(&backend, &master, "spec.md", "text/markdown", b"Name what sets Demi apart.").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root(FIRST, vec![say("Noted.")]);
    socket.send(&with_upload("m1", "The spec", &[(&spec, "spec.md")])).await;
    socket.until_idle().await;

    // The call waits for the file's bytes, and the user stops the turn.
    let hold = counts.hold_gets();
    scripts.root(FIRST, vec![shell("t1", "demi conversation history open 0/a1")]);
    socket.send(&send("m2", "Open the spec")).await;
    hold.until_waiting(1).await;
    socket.stop().await;
    hold.release();

    // The stopped call goes on at once and ends without a write; the next
    // turn's job, which starts after a model request, finds the scratch
    // directory without the file, and then saves it itself.
    scripts.root(
        FIRST,
        vec![
            shell("t2", "echo \"before: $(ls \"$DEMI_SCRATCH\")\" && demi conversation history open 0/a1"),
            say("opened"),
        ],
    );
    socket.chat("m3", "Open it now").await;
    let opened = result(&backend, &master, "t2").await;
    assert!(opened.contains("before: \n"), "the stopped call wrote: {opened}");
    assert!(opened.contains("/0-a1-spec.md"), "{opened}");
    backend.close().await;
}
