//! What fills the context (`compaction.md` § What fills the context): the
//! rows of the context card add up to the estimate the ring shows, with an
//! anchor, without one just after a compaction, and when the rows above
//! Messages alone exceed it.

use demi_agent_session::{PromptLayer, token_threshold};
use demi_host_interface::{CapabilityIndex, IndexGroup};
use demi_shared_types::{ContextRowKind, ContextUsage, InstructionEntry};

use super::*;

/// The window of `test_model`.
const WINDOW: u64 = 100_000;

/// A prompt with an identity, the tool rules, an index of two command
/// groups, the model line and a profile's added instructions.
fn prompt() -> SystemPrompt {
    let index = CapabilityIndex {
        groups: ["demi browser", "demi file"]
            .map(|path| IndexGroup {
                path: path.to_owned(),
                section: format!("{path}\nDoes what {path} does.\nOperations: open, read"),
            })
            .to_vec(),
    };
    let text = |text: &str| PromptLayer::Text(text.to_owned());
    SystemPrompt {
        layers: vec![
            text(&"You are Demi. ".repeat(40)),
            text(&"The shell tool: ".repeat(20)),
            PromptLayer::Commands(format!("Capabilities:\n\n{}", index.text())),
            text("This conversation runs on test-model."),
            text("Instructions for this subagent:\n\nYou find code and change nothing."),
        ],
    }
}

/// The instructions block of `personal` and the project's `AGENTS.md`.
fn instructions(personal: &str) -> NewContext {
    let entries = vec![
        InstructionEntry::Personal,
        InstructionEntry::File {
            path: "/workspace/AGENTS.md".to_owned(),
        },
    ];
    let mut text = "These are the instructions.".to_owned();
    for (entry, body) in entries.iter().zip([personal, "Run the tests."]) {
        text.push_str("\n\n");
        text.push_str(&entry.section(body));
    }
    NewContext {
        source: "instructions".to_owned(),
        text,
        instructions: entries,
    }
}

/// A skills catalog of two skills, as the skills plugin writes it.
fn skills() -> NewContext {
    let entry = |name: &str| {
        format!("  <skill>\n    <name>{name}</name>\n    <description>Does {name}.</description>\n    <location>/skills/{name}/SKILL.md</location>\n  </skill>\n")
    };
    NewContext {
        source: "skills".to_owned(),
        text: format!(
            "The following skills provide specialized instructions.\n\n<available_skills>\n{}{}</available_skills>",
            entry("release"),
            entry("web-design")
        ),
        instructions: Vec::new(),
    }
}

fn runtime(personal: &str) -> TestRuntime {
    let shell = tool("shell", |_| Box::pin(async { Ok(output("opened")) }));
    TestRuntime {
        execution: Some("Runs on the Cloud."),
        context: vec![instructions(personal), skills()],
        prompt: prompt(),
        ..test_runtime(vec![shell])
    }
}

fn row(usage: &ContextUsage, kind: ContextRowKind) -> Option<&demi_shared_types::ContextRow> {
    usage.rows.iter().find(|row| row.kind == kind)
}

/// The rows are in the card's order and add up: those before the buffer to
/// the estimate, or to the rows above Messages when those alone exceed it;
/// the buffer is the window less the threshold, and the free space the
/// rest. Only the buffer and the free space are exact.
fn assert_adds_up(usage: &ContextUsage) {
    let order = [
        ContextRowKind::Messages,
        ContextRowKind::SystemPrompt,
        ContextRowKind::Commands,
        ContextRowKind::Instructions,
        ContextRowKind::Skills,
        ContextRowKind::OtherContext,
        ContextRowKind::Summary,
        ContextRowKind::CompactionBuffer,
        ContextRowKind::FreeSpace,
    ];
    let positions: Vec<usize> = usage
        .rows
        .iter()
        .map(|row| order.iter().position(|kind| *kind == row.kind).unwrap())
        .collect();
    assert!(positions.is_sorted(), "{usage:#?}");
    let exact = |kind: &ContextRowKind| {
        matches!(kind, ContextRowKind::CompactionBuffer | ContextRowKind::FreeSpace)
    };
    let filled: u64 = usage
        .rows
        .iter()
        .filter(|row| !exact(&row.kind))
        .map(|row| row.tokens)
        .sum();
    let above: u64 = usage
        .rows
        .iter()
        .filter(|row| !exact(&row.kind) && row.kind != ContextRowKind::Messages)
        .map(|row| row.tokens)
        .sum();
    assert_eq!(filled, usage.tokens.max(above), "{usage:#?}");
    let buffer = WINDOW - token_threshold(WINDOW, None);
    assert_eq!(row(usage, ContextRowKind::CompactionBuffer).unwrap().tokens, buffer);
    assert_eq!(
        row(usage, ContextRowKind::FreeSpace).map_or(0, |row| row.tokens),
        WINDOW - filled - buffer
    );
    for row in &usage.rows {
        assert_eq!(row.estimated, !exact(&row.kind), "{row:?}");
        assert!(row.tokens > 0, "{row:?}");
    }
}

// Two sessions and four scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn the_rows_add_up_to_the_estimate_anchored_after_a_compaction_and_when_the_rest_exceeds_it() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("c1", "shell", json!({"script": "cd app && demi browser open http://localhost"})),
            event::response(1_000, 20),
        ]),
        Turn::Events(vec![event::text("The page opened."), event::response(60_000, 50)]),
        Turn::Events(vec![event::text("the summary"), event::response(10, 0)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start_on(&provider, runtime("Be brief."), &store, SessionConfig::default()).await;
    session.send(text("Open the page"), turn("t1")).unwrap().await.unwrap();

    // Anchored: Messages is the provider's count less the rows above it.
    let anchored = session.context_usage().await.unwrap();
    assert_eq!(anchored.tokens, 60_050);
    assert_adds_up(&anchored);
    for kind in [
        ContextRowKind::SystemPrompt,
        ContextRowKind::Commands,
        ContextRowKind::Instructions,
        ContextRowKind::Skills,
        ContextRowKind::OtherContext,
    ] {
        assert!(row(&anchored, kind).is_some(), "{kind:?}: {anchored:#?}");
    }
    assert!(row(&anchored, ContextRowKind::Summary).is_none());

    // Just after a compaction there is no anchor: the estimate is the
    // system prompt's, the tools' and the blocks' from the boundary on, and
    // the summary is a row.
    session.compact().unwrap().await.unwrap();
    let compacted = session.context_usage().await.unwrap();
    assert!(compacted.tokens < anchored.tokens, "{compacted:#?}");
    assert_adds_up(&compacted);
    assert!(row(&compacted, ContextRowKind::Summary).is_some(), "{compacted:#?}");

    // A provider that counts less than the rows above Messages: Messages is
    // empty, and the rows add up to more than the estimate.
    let provider = ScriptedRuntime::new([Turn::Events(vec![
        event::text("Done."),
        event::response(10, 0),
    ])]);
    let store = MemoryTreeStore::new();
    let long = "Keep it short. ".repeat(600);
    let session = start_on(&provider, runtime(&long), &store, SessionConfig::default()).await;
    session.send(text("Hello"), turn("t1")).unwrap().await.unwrap();
    let exceeded = session.context_usage().await.unwrap();
    assert_eq!(exceeded.tokens, 10);
    assert_adds_up(&exceeded);
    assert!(row(&exceeded, ContextRowKind::Messages).is_none(), "{exceeded:#?}");
}
