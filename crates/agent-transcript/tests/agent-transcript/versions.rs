//! Versions in the live transcript (`message-editing.md` § Versions): what
//! a compaction carries past its boundary keeps its versions, in the log
//! and in the patch that tells the page.

use std::collections::BTreeMap;
use std::{rc::Rc, sync::Arc};

use demi_agent_store::testing::test_model;
use demi_agent_transcript::{TranscriptLog, testing::SequentialIds};
use demi_conversation_socket_protocol::{TranscriptPatch, Versions};
use demi_shared_types::{Block, TextBlock, Timestamp};

// Cost: pure, under a millisecond.

struct Fixed;

impl demi_shared_types::Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp::UNIX_EPOCH
    }
}

fn said(text: &str) -> Block {
    Block::Text(TextBlock {
        id: text.try_into().unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        text: text.to_owned(),
        forkable: true,
        entries: Vec::new(),
    })
}

const SECOND_OF_TWO: Versions = Versions { index: 2, count: 2 };

#[test]
fn a_compaction_boundary_leaves_the_versions_with_the_message_it_moves() {
    let blocks = vec![said("u1"), said("a1"), said("u2"), said("a2")];
    let mut log = TranscriptLog::new(blocks, Rc::new(SequentialIds::new("b")), Arc::new(Fixed))
        .with_versions(BTreeMap::from([(2, SECOND_OF_TWO)]));

    log.insert_compaction_boundary(2, &test_model(), "summary".into(), 10, Vec::new());

    assert_eq!(log.versions(), &BTreeMap::from([(3, SECOND_OF_TWO)]));
    let batch = log.take_patches().expect("the boundary is a change");
    let added: Vec<_> = batch
        .patches
        .iter()
        .filter_map(|patch| match patch {
            TranscriptPatch::Add { index, versions, .. } => Some((*index, *versions)),
            _ => None,
        })
        .collect();
    // The page inserts the boundary before the message, which keeps the
    // versions it holds.
    assert_eq!(added, [(2, None)]);
}
