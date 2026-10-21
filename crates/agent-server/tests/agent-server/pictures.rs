//! The pictures of the Host a reply shows, kept with the reply
//! (`crates/agent-server/src/node/reply_files.rs`,
//! `docs/decisions/0046-a-reply-keeps-the-pictures-it-shows.md`): named as
//! being copied in the step that completes the text, each settled as its copy
//! is in or as why it could not be made, copied again when the node starts if
//! the backend stopped first, and never copied for a text that only comes
//! back into the window.
//!
//! The Host is a test's own: it copies the paths it holds, and holds each copy
//! until the test lets it go.

use demi_agent_server::ServerConfig;
use demi_agent_store::testing::MemoryTreeStore;
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame, TranscriptPatch};
use demi_provider_common::testing::ScriptedRuntime;
use demi_shared_types::{Block, PictureCopy, PictureFailure, ReplyPicture};

use crate::editing::{edit, said, user_block};
use crate::support::{Fixture, TestProduct, send, session_of, until};

// Cost: a scripted turn each, a few milliseconds.

fn copying(target: &str) -> ReplyPicture {
    ReplyPicture { target: target.into(), copy: PictureCopy::Copying }
}

/// The pictures of the last text of the conversation's root.
fn pictures_of(fixture: &Fixture) -> Vec<ReplyPicture> {
    session_of(fixture)
        .transcript()
        .blocks
        .iter()
        .rev()
        .find_map(|block| match block {
            Block::Text(text) => Some(text.pictures.clone().unwrap_or_default()),
            _ => None,
        })
        .unwrap_or_default()
}

/// A fixture whose Host copies `out/chart.png` as attachment `a1`, refuses
/// `notes.md` as no image and has nothing else, with its copies held.
fn held_host(script: &ScriptedRuntime, store: std::rc::Rc<MemoryTreeStore>) -> Fixture {
    let fixture = Fixture::with_product(script, TestProduct::default(), store, ServerConfig::default());
    let files = &fixture.product.host_files;
    files.pictures.borrow_mut().insert("out/chart.png".into(), Ok("a1".into()));
    files.pictures.borrow_mut().insert(
        "notes.md".into(),
        Err(PictureFailure::NotAnImage { media_type: "text/markdown".into() }),
    );
    files.hold();
    fixture
}

const REPLY: &str = "![chart](out/chart.png) ![again](out/chart.png) ![gone](missing.png) ![notes](notes.md) ![web](https://example.com/x.png)";

// The step that completes the reply carries its pictures as being copied, so
// a page that sees the reply complete never shows a picture as a link first.
// Each picture then settles as its copy is in, or as the cause the Host gave:
// the file is not there, or is no image. A path the reply wrote twice is
// copied once.
//
// Planted defects it catches: pictures named by a later step than the one
// that completes the text, a reason written as a sentence, and a path copied
// twice.
#[tokio::test(flavor = "local")]
async fn a_reply_names_its_pictures_as_it_completes_and_settles_each_as_its_copy_is_in() {
    let script = ScriptedRuntime::new([said(REPLY)]);
    let fixture = held_host(&script, MemoryTreeStore::new());
    let mut client = fixture.opened().await;
    client.send(send("m1", "show me")).await;
    let frames = client.until_idle().await;

    let completed: Vec<_> = frames
        .iter()
        .flat_map(|frame| match frame {
            ServerFrame::TranscriptPatch { patches, .. } => patches.clone(),
            _ => Vec::new(),
        })
        .filter_map(|patch| match patch {
            TranscriptPatch::Add { value: Block::Text(text), .. } | TranscriptPatch::ReplaceBlock { value: Block::Text(text), .. }
                if text.forkable =>
            {
                Some(text)
            }
            _ => None,
        })
        .collect();
    assert!(!completed.is_empty());
    for text in &completed {
        assert_eq!(
            text.pictures,
            Some(vec![copying("out/chart.png"), copying("missing.png"), copying("notes.md")]),
            "a complete text always carries its pictures"
        );
    }

    fixture.product.host_files.release();
    until(|| pictures_of(&fixture).iter().all(|picture| picture.copy != PictureCopy::Copying)).await;
    assert_eq!(
        pictures_of(&fixture),
        [
            ReplyPicture { target: "out/chart.png".into(), copy: PictureCopy::Copied { attachment: "a1".into() } },
            ReplyPicture { target: "missing.png".into(), copy: PictureCopy::Failed { reason: PictureFailure::Missing } },
            ReplyPicture {
                target: "notes.md".into(),
                copy: PictureCopy::Failed { reason: PictureFailure::NotAnImage { media_type: "text/markdown".into() } }
            },
        ]
    );
    assert_eq!(*fixture.product.host_files.copied.borrow(), ["out/chart.png", "missing.png", "notes.md"]);
}

// The backend stops while a copy is out: the reply is saved with its picture
// still being copied, and the node that starts again copies it, once.
//
// Planted defect it catches: a node that starts leaves the picture as
// "copying" for good.
#[tokio::test(flavor = "local")]
async fn a_picture_not_settled_when_the_backend_stopped_is_copied_when_the_node_starts() {
    let script = ScriptedRuntime::new([said("![chart](out/chart.png)")]);
    let fixture = held_host(&script, MemoryTreeStore::new());
    let mut client = fixture.opened().await;
    client.send(send("m1", "show me")).await;
    client.until_idle().await;
    until(|| fixture.product.host_files.copied.borrow().len() == 1).await;
    client.send(ClientFrame::Close {}).await;
    client.next_until(|frame| *frame == ServerFrame::Closed).await;
    let stored = fixture.store.copy();

    let after = held_host(&ScriptedRuntime::new(Vec::new()), stored);
    after.product.host_files.release();
    let reopened = after.client();
    reopened.open().await;
    until(|| pictures_of(&after).iter().all(|picture| picture.copy != PictureCopy::Copying) && !pictures_of(&after).is_empty()).await;
    assert_eq!(
        pictures_of(&after),
        [ReplyPicture { target: "out/chart.png".into(), copy: PictureCopy::Copied { attachment: "a1".into() } }]
    );
    assert_eq!(*after.product.host_files.copied.borrow(), ["out/chart.png"]);
}

// A reply that comes back into the window, as an earlier version does when the
// page switches to it, is not copied: only a reply the node saw written is.
// Here the earlier version's copy was still out when the backend stopped, so
// it stays as it was.
//
// Planted defect it catches: a copy started for every complete text that
// arrives in a patch.
#[tokio::test(flavor = "local")]
async fn a_reply_that_comes_back_with_an_earlier_version_is_not_copied() {
    let script = ScriptedRuntime::new([said("![chart](out/chart.png)"), said("second try")]);
    let fixture = held_host(&script, MemoryTreeStore::new());
    let mut client = fixture.opened().await;
    client.send(send("m1", "show me")).await;
    client.until_idle().await;
    until(|| fixture.product.host_files.copied.borrow().len() == 1).await;
    let version = session_of(&fixture).transcript().version;
    let asked = user_block(&fixture, "m1");
    client
        .send(edit("edit-1", &asked, &version, demi_agent_server::testing::client_text("show me again")))
        .await;
    client.until_idle().await;
    client.send(ClientFrame::Close {}).await;
    client.next_until(|frame| *frame == ServerFrame::Closed).await;
    let stored = fixture.store.copy();

    let after = held_host(&ScriptedRuntime::new(Vec::new()), stored);
    after.product.host_files.release();
    let mut reopened = after.client();
    reopened.open().await;
    reopened.next_until(crate::support::is_root_reset).await;
    // The first version's reply is out of the window: the node has nothing to
    // copy now.
    assert!(pictures_of(&after).is_empty(), "{:?}", pictures_of(&after));
    let transcript = session_of(&after).transcript();
    let place = transcript.blocks[0].id().clone();
    reopened
        .send(ClientFrame::SwitchVersion { block_id: place, to: 1, version: transcript.version })
        .await;
    reopened.until_idle().await;
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
    assert_eq!(pictures_of(&after), [copying("out/chart.png")]);
    assert!(after.product.host_files.copied.borrow().is_empty());
}
