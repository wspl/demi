//! The run of steps each block joins as the transcript stores it: a page
//! groups by it and judges nothing.

use std::{rc::Rc, sync::Arc};

use demi_agent_store::testing::{test_model, text};
use demi_agent_transcript::{TranscriptLog, testing::SequentialIds};
use demi_provider_common::ToolCall;
use demi_shared_types::{
    AgentMessage, AgentMessageEvent, BlockId, CommandId, CommandReport, CompletionOutcome, NodeId,
    PermissionOutcome, ReportEvent, Sender, Timestamp, TokenUsage, TurnId, WakeupPlacement,
};

// Cost: pure, under a millisecond.

struct Fixed;

impl demi_shared_types::Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp::UNIX_EPOCH
    }
}

fn log() -> TranscriptLog {
    TranscriptLog::new(Vec::new(), Rc::new(SequentialIds::new("b")), Arc::new(Fixed))
}

fn turn() -> TurnId {
    TurnId::try_from("t").unwrap()
}

fn call(log: &mut TranscriptLog, name: &str) {
    log.push_tool_call(
        &test_model(),
        ToolCall {
            tool_use_id: name.into(),
            tool_name: "shell".into(),
            input: serde_json::json!({ "script": "true", "description": name }),
        },
    );
}

/// A command's report: its end, or its progress alone.
fn report(log: &mut TranscriptLog, id: &str, ended: bool) {
    let event = if ended {
        ReportEvent::Ended { exit_code: Some(0) }
    } else {
        ReportEvent::Running {
            running_ms: 1,
            idle_ms: 0,
        }
    };
    log.push_wakeup(
        BlockId::try_from(id).unwrap(),
        turn(),
        &test_model(),
        WakeupPlacement::Steer,
        vec![CommandReport {
            command_id: CommandId::try_from("17").unwrap(),
            title: "Build".into(),
            event,
            output: String::new(),
            media: Vec::new(),
        }],
    );
}

/// A message to the root: another agent's, or the user's decision. The
/// sender is the root's child, so below it.
fn message(log: &mut TranscriptLog, id: &str, event: AgentMessageEvent) {
    message_from(log, id, event, true);
}

/// A message to the node, from a sender below it, `sender_below`, or above or
/// beside it.
fn message_from(log: &mut TranscriptLog, id: &str, event: AgentMessageEvent, sender_below: bool) {
    let sender = matches!(event, AgentMessageEvent::Message {} | AgentMessageEvent::Completion { .. })
        .then(|| Sender {
            id: NodeId::try_from("child").unwrap(),
            number: 1,
            description: "Tests".into(),
            round: 1,
        });
    log.push_agent_message(
        turn(),
        &test_model(),
        AgentMessage {
            id: BlockId::try_from(id).unwrap(),
            sender,
            recipient_id: NodeId::try_from("root").unwrap(),
            timestamp: Timestamp::UNIX_EPOCH,
            content: "Done.".into(),
            event,
            origin: None,
        },
        sender_below,
    );
}

/// Each block's run, as the position of the run's first block; none for a
/// block that is not a step.
fn runs(log: &TranscriptLog) -> Vec<Option<usize>> {
    let blocks = log.blocks();
    blocks
        .iter()
        .map(|block| {
            block
                .run()
                .map(|run| blocks.iter().position(|first| first.id() == run).unwrap())
        })
        .collect()
}

#[test]
fn calls_notices_and_receipts_are_one_run_until_what_the_reader_reads() {
    // The agent runs three commands, a command it started earlier reports
    // its end, a child reports that it finished, and the agent runs two
    // more: one run. A report of progress alone, a context block and a
    // request's usage pass through it.
    let mut log = log();
    log.push_user(turn(), &test_model(), text("Fix the tests"), Vec::new(), None, None);
    log.open_thinking(&test_model(), "Plan.".into());
    call(&mut log, "a");
    call(&mut log, "b");
    log.push_response(&test_model(), Some(TokenUsage::default()));
    call(&mut log, "c");
    report(&mut log, "progress", false);
    report(&mut log, "end", true);
    message(
        &mut log,
        "child-end",
        AgentMessageEvent::Completion {
            outcome: CompletionOutcome::Completed,
        },
    );
    log.push_context(turn(), &test_model(), "host".into(), "Moved.".into(), Vec::new(), None);
    call(&mut log, "d");
    call(&mut log, "e");
    assert_eq!(
        runs(&log),
        [None, Some(1), Some(1), Some(1), None, Some(1), None, Some(1), Some(1), None, Some(1), Some(1)]
    );
}

#[test]
fn text_the_users_words_and_decisions_end_a_run() {
    let mut log = log();
    log.push_user(turn(), &test_model(), text("Go"), Vec::new(), None, None);
    call(&mut log, "a");
    log.append_text(&test_model(), "Checked.");
    call(&mut log, "b");
    report(&mut log, "end", true);
    message(
        &mut log,
        "permission:1",
        AgentMessageEvent::Permission {
            outcome: PermissionOutcome::Allowed,
            action: "manage skills".into(),
            categories: vec!["skills.manage".into()],
            credential: None,
            host: None,
            pairing: false,
        },
    );
    call(&mut log, "c");
    log.push_steer(BlockId::try_from("steer").unwrap(), turn(), &test_model(), text("Also the docs"), Vec::new());
    message(&mut log, "update", AgentMessageEvent::Message {});
    log.append_text(&test_model(), "Done.");
    assert_eq!(
        runs(&log),
        [None, Some(1), None, Some(3), Some(3), None, Some(6), None, Some(8), None]
    );
}

#[test]
fn a_message_from_above_or_beside_ends_a_run_and_one_from_below_joins_it() {
    // A child runs two commands, and its parent tells it to keep the old
    // heading: the parent's message ends the run, as the user's words do. It
    // runs another command, and its own child sends an update and then
    // finishes between two calls: both are steps of the run.
    let mut log = log();
    log.push_user(turn(), &test_model(), text("Tidy the notes"), Vec::new(), None, None);
    call(&mut log, "a");
    call(&mut log, "b");
    message_from(&mut log, "from-parent", AgentMessageEvent::Message {}, false);
    call(&mut log, "c");
    message_from(&mut log, "from-child", AgentMessageEvent::Message {}, true);
    call(&mut log, "d");
    message(
        &mut log,
        "child-end",
        AgentMessageEvent::Completion {
            outcome: CompletionOutcome::Completed,
        },
    );
    assert_eq!(
        runs(&log),
        [None, Some(1), Some(1), None, Some(4), Some(4), Some(4), Some(4)]
    );
}

#[test]
fn redacted_reasoning_is_a_step_of_its_run() {
    // The model's reasoning arrives redacted, then it thinks and runs a
    // command: one run, named after the redacted reasoning that opened it.
    let mut log = log();
    log.push_user(turn(), &test_model(), text("Go"), Vec::new(), None, None);
    log.push_redacted_thinking(&test_model(), "opaque".into());
    log.open_thinking(&test_model(), "Plan.".into());
    call(&mut log, "a");
    assert_eq!(runs(&log), [None, Some(1), Some(1), Some(1)]);
}

#[test]
fn the_steps_after_the_systems_resume_join_the_run_before_it() {
    // A deploy command is lost when the backend restarts, the turn
    // resumes, and the agent runs four more commands: one run.
    let mut log = log();
    log.push_user(turn(), &test_model(), text("Deploy"), Vec::new(), None, None);
    call(&mut log, "deploy");
    log.push_resume(turn(), &test_model(), Some("The backend restarted.".into()));
    report(&mut log, "progress", false);
    call(&mut log, "b");
    call(&mut log, "c");
    call(&mut log, "d");
    call(&mut log, "e");
    assert_eq!(
        runs(&log),
        [None, Some(1), None, None, Some(1), Some(1), Some(1), Some(1)]
    );
}

#[test]
fn the_steps_after_a_continued_stop_join_the_run_before_it() {
    // The agent runs a command, the user stops it and continues: the next
    // call joins the run, and after a Stop the user did not continue, a
    // command's end that arrives starts a run of its own.
    let mut log = log();
    log.push_user(turn(), &test_model(), text("Go"), Vec::new(), None, None);
    call(&mut log, "a");
    log.push_abort(&test_model());
    log.mark_latest_abort_resumed();
    log.push_resume(turn(), &test_model(), None);
    call(&mut log, "b");
    log.push_abort(&test_model());
    report(&mut log, "end", true);
    assert_eq!(runs(&log), [None, Some(1), None, None, Some(1), None, Some(6)]);
}
