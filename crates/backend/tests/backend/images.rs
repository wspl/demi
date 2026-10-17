//! The agent's `demi image` (`images.md`, `media.md`), the `image` plugin
//! over the port's Providers service: an image made to a file and to `demi
//! file view`, an edit of the Host's files, warnings for what the result
//! did not honour, a used-up account under Balanced moving the call to
//! another, a usage ledger row per call, a stopped call's cancelled row, no
//! account named to a shared instance's user, an entry whose Image
//! Generation is off never chosen, the entry a call takes: subscriptions
//! before keys, each in the order of Settings › Providers, the next once
//! one is used up, never another after a refusal, and the one
//! `--provider` names or why it cannot; what `demi image models` says of
//! each model; and a compatible API whose shape is OpenAI Images. The
//! model is the scripted family of the subagent scenarios, the image role a
//! scripted family's, or OpenAI Images' requests to a scripted server, and
//! the device a real runner. No test calls a real model.

use std::sync::Arc;

use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_shared_types::B64Bytes;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::providers::{Accounts, LoginState, ModelCatalog, ProviderAnswer, ProviderDto, Providers};
use reqwest::StatusCode;
use serde_json::json;

use crate::accounts::{awaited, device_entry, ended, start_login};
use crate::agent_profiles::{assert_shows, last_root};
use crate::conversations::{FIRST, SECOND, Socket, choose, create, working_on};
use demi_web_api_protocol::auth::Role;
use crate::families::{Directory, ImageScript, LoginScript, QuotaScript, ScriptedKey, ScriptedSubscription};
use crate::subagents::{Scripts, say, shell, tree_among};
use crate::support::{Harness, Session, TestBackend};
use demi_web_api_protocol::state::SyncEvent;
use crate::usage::{Row, rows};

/// The scripted family's image models, the older first, so the list shows
/// the newer first; the test backend's catalog says what each is
/// (`crate::support::MEDIA_CATALOG`).
const PICTURES: &[&str] = &["picture-1", "picture-2"];

/// A second family's image model.
const DRAWINGS: &[&str] = &["drawing-1"];

/// A `device` family whose accounts make images of `models` as `images`
/// scripts them, and the scripts of its logins.
fn image_family(images: &Arc<ImageScript>) -> crate::accounts::Scripts {
    let login = Arc::new(LoginScript::default());
    let quota = Arc::new(QuotaScript::free(Vec::new()));
    let family = ScriptedSubscription {
        login: login.clone(),
        quota: quota.clone(),
        directory: Arc::new(Directory::default()),
        runs: None,
        process_host: false,
        images: Some(images.clone()),
    };
    crate::accounts::Scripts {
        login,
        quota,
        families: demi_backend::families::builtin().with("device", family),
    }
}

/// The ledger's image rows.
fn image_rows(harness: &Harness) -> Vec<Row> {
    rows(harness)
        .into_iter()
        .filter(|row| row.purpose.as_deref() == Some("image"))
        .collect()
}

// A few seconds: three real devices run the shell jobs of six turns, which
// pipe an image of a quarter megabyte through the call's stdout.
//
// Planted defects: an image written to stdout cut or changed, so `cmp`
// fails and `demi file view` shows no image; an edit that reads no file
// of the Host, or reads the primary Host's inside `demi host shell` on an
// attached one; a stopped call that writes no row; the administrator's
// account named to a user of the shared entry; a row counted under no
// turn.
#[tokio::test]
async fn an_agent_makes_an_image_to_a_file_and_to_file_view_and_edits_the_hosts_files() {
    let made = demi_agent_store::testing::speckled(700, 700).into_bytes();
    // The vendor lists an alias too, as Codex lists gpt-image-2 beside its
    // dated name: the alias is a name of picture-1, not a row of its own.
    let images = Arc::new(ImageScript::new(&["picture-1", "picture-latest", "picture-2"], made.clone()));
    let logins = image_family(&images);
    let scripts = Arc::new(Scripts::default());
    let harness = Harness::new().with_file_package();
    let (harness, backend, master, _device, root, tree) =
        tree_among(&scripts, harness, logins.families.clone()).await;
    // The model reads images, so `demi file view` shows it one.
    let model = |id: &str, name: &str| {
        json!({ "id": id, "displayName": name, "contextWindow": 100000, "outputLimit": null,
            "thinkingEfforts": [], "acceptedExtensions": ["png", "jpg"], "fastTier": null })
    };
    let reads = backend
        .patch(&format!("/api/providers/{tree}"), &master, json!({ "models": [model("m", "M"), model("n", "N")] }))
        .await;
    assert_eq!(reads.status, StatusCode::OK, "{}", String::from_utf8_lossy(&reads.body));
    choose(&backend, &master, FIRST, &tree, "m").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // Before any sign-in, nothing makes images.
    scripts.root(
        FIRST,
        vec![shell("t1", "demi image generate 'a red fox' --model picture-1; echo exit=$?"), say("none")],
    );
    socket.chat("m1", "Draw a fox").await;
    assert_shows(
        &last_root(&scripts),
        "demi image generate: no provider makes images; sign in to ChatGPT or Grok, or add an OpenAI, xAI or Google key, in Settings › Providers\nexit=1",
    );

    logins.login.signs_in("first");
    let entry = device_entry(&backend, &master, &logins).await;
    let named = backend.patch(&format!("/api/providers/{}", entry.id), &master, json!({ "label": "Sub" })).await;
    assert_eq!(named.status, StatusCode::OK, "{}", String::from_utf8_lossy(&named.body));
    std::fs::write(format!("{root}/expected.png"), &made).unwrap();
    let photo = demi_agent_store::testing::png(4, 3, 1).into_bytes();
    std::fs::write(format!("{root}/photo.png"), &photo).unwrap();
    std::fs::write(format!("{root}/notes.txt"), "not an image\n").unwrap();
    scripts.root(
        FIRST,
        vec![
            shell(
                "t2",
                &[
                    "demi image generate 'a red fox' --model picture-1 > fox.png && cmp fox.png expected.png && echo same",
                    "demi image generate 'a red fox' --model picture-1 --aspect 16:9 --transparent | demi file view",
                    "printf 'make the ears rounder' | demi image edit - fox.png photo.png --model picture-1 > fox-2.png; echo exit=$?",
                    "demi image edit 'x' notes.txt --model picture-1; echo exit=$?",
                    "demi image generate 'x' --model nope; echo exit=$?",
                    "demi image generate 'x' --model picture-2 > /dev/null",
                    "demi image models",
                ]
                .join("\n"),
            ),
            say("drawn"),
        ],
    );
    socket.chat("m2", "Draw it now").await;
    let asked = last_root(&scripts);
    let said = "700×700 PNG from picture-1 (Sub, first@example.test)\n";
    for shown in [
        &format!("{said}same\n"),
        &format!("[image 1: image/png, 700 × 700 px, {} bytes]", made.len()),
        &format!(
            "{said}demi image generate: warning: asked for 16:9, got 700×700\ndemi image generate: warning: asked for a transparent background, got an opaque image\n"
        ),
        &format!("{said}exit=0\n"),
        "demi image edit: notes.txt: not a PNG, JPEG or WebP image\nexit=1\n",
        "demi image generate: no provider makes \"nope\"; demi image models lists them\nexit=1\n",
        "700×700 PNG from picture-2 (Sub, first@example.test)\n",
        "ID         VENDOR  RELEASED    ENTRY  NAME\npicture-2  OpenAI  2026-06-01  Sub    Picture 2\npicture-1  OpenAI  2026-01-05  Sub    Picture 1",
    ] {
        assert_shows(&asked, shown);
    }
    assert_eq!(std::fs::read(format!("{root}/fox-2.png")).unwrap(), made);

    // Inside `demi host shell` on an attached Host, an edit reads that
    // Host's file, beside the far shell's directory.
    let alpha = backend.pair(&master, "alpha").await;
    harness.attach(FIRST, alpha.id());
    let far = demi_agent_store::testing::png(5, 2, 9).into_bytes();
    std::fs::write(alpha.runner.home_dir().join("far.png"), &far).unwrap();
    scripts.root(
        FIRST,
        vec![
            shell("t3", "demi host shell --host alpha 'demi image edit \"keep it\" far.png --model picture-1 > far-2.png'; echo exit=$?"),
            say("edited there"),
        ],
    );
    socket.chat("m3", "Edit the far one").await;
    assert_shows(&last_root(&scripts), "700×700 PNG from picture-1 (Sub, first@example.test)\nexit=0");
    let calls = images.calls();
    let sent: Vec<&[u8]> = calls.last().unwrap().1.images.iter().map(|image| &image.bytes[..]).collect();
    assert_eq!(sent, [&far[..]]);
    assert_eq!(std::fs::read(alpha.runner.home_dir().join("far-2.png")).unwrap(), made);

    // The vendor was asked as the agent asked, an edit with the Host's
    // files as they are.
    let calls = images.calls();
    let asked: Vec<_> = calls
        .iter()
        .map(|(_, request)| (request.model.as_str(), request.prompt.as_str(), request.aspect.map(|aspect| aspect.to_string()), request.transparent))
        .collect();
    assert_eq!(
        asked,
        [
            ("picture-1", "a red fox", None, false),
            ("picture-1", "a red fox", Some("16:9".to_owned()), true),
            ("picture-1", "make the ears rounder", None, false),
            ("picture-2", "x", None, false),
            ("picture-1", "keep it", None, false),
        ]
    );
    let edited: Vec<(&str, &[u8])> = calls[2]
        .1
        .images
        .iter()
        .map(|image| (image.media_type, &image.bytes[..]))
        .collect();
    assert_eq!(edited, [("image/png", &made[..]), ("image/png", &photo[..])]);

    // Each call is a ledger row of the root agent, its account and model,
    // with the vendor's counts.
    let account = backend
        .get(&format!("/api/providers/{}/accounts", entry.id), Some(&master))
        .await
        .json::<Accounts>()
        .active
        .unwrap()
        .to_string();
    let rows = image_rows(&harness);
    assert_eq!(rows.len(), 5, "{rows:?}");
    let row = &rows[0];
    assert_eq!(
        (row.agent, row.turn_id.as_deref(), row.provider_id.as_str(), row.account_id.as_deref(), row.model_id.as_str()),
        (Some(0), Some("m2"), entry.id.as_str(), Some(account.as_str()), "picture-1")
    );
    // Each row is counted under the turn whose shell call ran the command,
    // the far Host's too.
    let turns: Vec<Option<&str>> = rows.iter().map(|row| row.turn_id.as_deref()).collect();
    assert_eq!(turns, [Some("m2"), Some("m2"), Some("m2"), Some("m2"), Some("m3")]);
    assert_eq!(
        (row.vendor_id.as_deref(), row.tokens, row.outcome.as_deref()),
        (Some("gen-1"), [Some(25), Some(515), Some(0), Some(0)], Some("answered"))
    );
    assert!(row.request_id.is_some() && row.started_at.is_some_and(|started| started <= row.ended_at));

    // A call whose job is stopped ends the vendor's request and leaves a
    // cancelled row.
    images.hold.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut counts = images.counts.subscribe();
    let asked = counts.borrow().0;
    scripts.root(
        FIRST,
        vec![
            shell(
                "t4",
                "demi image generate 'a fox' --model picture-1 > held.png & call=$!; until [ -f go ]; do sleep 0.05; done; kill $call; wait $call; echo stopped",
            ),
            say("stopped"),
        ],
    );
    socket.send(&crate::conversations::send("m4", "Draw and stop")).await;
    counts.wait_for(|(now, _)| *now > asked).await.unwrap();
    std::fs::write(format!("{root}/go"), "").unwrap();
    socket.until_idle().await;
    counts.wait_for(|(_, dropped)| *dropped == 1).await.unwrap();
    crate::support::eventually("the stopped call's row is written", async || {
        image_rows(&harness).last().and_then(|row| row.outcome.clone()).as_deref() == Some("cancelled")
    })
    .await;
    assert_eq!(image_rows(&harness).last().unwrap().turn_id.as_deref(), Some("m4"));
    images.hold.store(false, std::sync::atomic::Ordering::SeqCst);

    // Someone who infers with a shared instance's entry is not told whose
    // account made the image.
    harness.add_user("reader@example.test", "reader-pass-1", Role::User);
    let reader = backend.login("reader@example.test", "reader-pass-1").await;
    create(&backend, &reader, SECOND).await;
    choose(&backend, &reader, SECOND, &tree, "m").await;
    let reading = backend.pair(&reader, "reader-laptop").await;
    working_on(&harness, &reading, SECOND);
    let mut theirs = Socket::connect(&backend, &reader, SECOND).await;
    theirs.open().await;
    scripts.root(SECOND, vec![shell("r1", "demi image generate 'a fox' --model picture-1 > fox.png"), say("drawn")]);
    theirs.chat("m1", "Draw a fox").await;
    let asked = scripts.asked(|session| session == SECOND).pop().unwrap();
    assert_shows(&asked, "700×700 PNG from picture-1 (Sub)");
    backend.close().await;
}

// About a second: two logins and a real device's shell jobs of two turns.
//
// Planted defect: a used-up account the call does not mark, so Balanced
// takes it again and the call fails instead of moving.
#[tokio::test]
async fn a_used_up_account_under_balanced_moves_the_call_to_the_other_and_none_left_fails_in_words() {
    let made = demi_agent_store::testing::png(8, 8, 2).into_bytes();
    let images = Arc::new(ImageScript::new(&PICTURES[..1], made.clone()));
    let logins = image_family(&images);
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _device, _root, _tree) =
        tree_among(&scripts, Harness::new(), logins.families.clone()).await;
    logins.login.signs_in("first");
    let entry = device_entry(&backend, &master, &logins).await;
    let path = format!("/api/providers/{}/accounts", entry.id);
    logins.login.signs_in("second");
    let login = start_login(&backend, &master, &format!("{path}/login"), json!({})).await;
    let LoginState::Completed { credential_id: second, .. } = awaited(&backend, &master, &login, ended).await
    else {
        panic!("the second login did not complete");
    };
    let first = backend.get(&path, Some(&master)).await.json::<Accounts>().active.unwrap();
    let rotated = backend
        .patch(&format!("/api/providers/{}", entry.id), &master, json!({ "rotation": "balanced" }))
        .await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", String::from_utf8_lossy(&rotated.body));
    *images.used_up.lock().unwrap() = vec![first.to_string()];
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    scripts.root(
        FIRST,
        vec![shell("t1", "demi image generate 'a fox' --model picture-1 > fox.png; echo exit=$?"), say("drawn")],
    );
    socket.chat("m1", "Draw a fox").await;
    assert_shows(&last_root(&scripts), &format!("8×8 PNG from picture-1 ({}, second@example.test)\nexit=0", entry.label));
    let accounts: Vec<(Option<String>, Option<String>)> = image_rows(&harness)
        .into_iter()
        .map(|row| (row.account_id, row.outcome))
        .collect();
    assert_eq!(
        accounts,
        [
            (Some(first.to_string()), Some("usage_limit".to_owned())),
            (Some(second.to_string()), Some("answered".to_owned())),
        ]
    );

    // With every account used up, the call fails and names the vendor.
    images.used_up.lock().unwrap().push(second.to_string());
    scripts.root(
        FIRST,
        vec![shell("t2", "demi image generate 'a fox' --model picture-1 > fox.png; echo exit=$?"), say("none")],
    );
    socket.chat("m2", "Again").await;
    let asked = last_root(&scripts);
    assert_shows(&asked, "demi image generate: Scripted's usage limit is reached, resetting at ");
    assert_shows(&asked, "\nexit=1");
    backend.close().await;
}

// About a second: two logins and a real device's shell jobs of two turns.
//
// Planted defects: a switch stored for an entry that makes no images, or a
// shape for a vendor's; an entry whose Image Generation is off still
// chosen, or a model only it offers listed by `demi image models`; a
// --model of an entry that is off told as a model nobody makes; every
// entry off told as none signed in.
#[tokio::test]
async fn an_entry_with_image_generation_off_is_never_chosen_and_says_why() {
    let made = demi_agent_store::testing::png(8, 8, 2).into_bytes();
    let first_images = Arc::new(ImageScript::new(&PICTURES[..1], made.clone()));
    let second_images = Arc::new(ImageScript::new(DRAWINGS, made.clone()));
    let logins = image_family(&first_images);
    let second = ScriptedSubscription {
        login: logins.login.clone(),
        quota: logins.quota.clone(),
        directory: Arc::new(Directory::default()),
        runs: None,
        process_host: false,
        images: Some(second_images.clone()),
    };
    let families = logins.families.clone().with("device-2", second);
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _device, _root, tree) =
        tree_among(&scripts, Harness::new(), families).await;
    // An entry that makes no images has no switch to set, and a vendor's
    // entry makes images as its vendor does.
    let refused = backend
        .patch(&format!("/api/providers/{tree}"), &master, json!({ "media": { "image": { "enabled": false } } }))
        .await;
    assert_eq!(refused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
    let fixed = backend
        .patch(&format!("/api/providers/{tree}"), &master, json!({ "media": { "image": { "shape": "openai-images" } } }))
        .await;
    assert_eq!(fixed.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::ShapeFixed));
    logins.login.signs_in("first");
    let first = device_entry(&backend, &master, &logins).await;
    let login = start_login(
        &backend,
        &master,
        "/api/providers/subscription-login",
        json!({ "providerType": "device-2" }),
    )
    .await;
    let LoginState::Completed { provider_id: second, .. } = awaited(&backend, &master, &login, ended).await
    else {
        panic!("the second entry's login did not complete");
    };
    let set = async |id: &str, body: serde_json::Value| {
        let answer = backend.patch(&format!("/api/providers/{id}"), &master, body).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.body));
        answer.json::<ProviderAnswer>().provider
    };
    set(first.id.as_str(), json!({ "label": "First" })).await;
    set(second.as_str(), json!({ "label": "Second" })).await;
    let off = set(first.id.as_str(), json!({ "media": { "image": { "enabled": false } } })).await;
    assert!(!off.media.image.enabled);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The first entry is off: the call goes to the second, the list shows
    // only the second's, and the first's model says why.
    scripts.root(
        FIRST,
        vec![
            shell(
                "t1",
                &[
                    "demi image generate 'a fox' --model drawing-1 > /dev/null",
                    "demi image models",
                    "demi image generate 'a fox' --model picture-1; echo exit=$?",
                ]
                .join("\n"),
            ),
            say("drawn"),
        ],
    );
    socket.chat("m1", "Draw a fox").await;
    let asked = last_root(&scripts);
    for shown in [
        "8×8 PNG from drawing-1 (Second, first@example.test)\nID         VENDOR  RELEASED    ENTRY   NAME\ndrawing-1  OpenAI  2026-02-01  Second  Drawing 1\ndemi image generate",
        "demi image generate: \"picture-1\" is First's, whose image generation is off in Settings › Providers\nexit=1",
    ] {
        assert_shows(&asked, shown);
    }
    assert!(first_images.calls().is_empty());

    // Both off: no entry makes images, and the list says why under its
    // header.
    set(second.as_str(), json!({ "media": { "image": { "enabled": false } } })).await;
    scripts.root(
        FIRST,
        vec![shell("t2", "demi image models; echo exit=$?"), say("none")],
    );
    socket.chat("m2", "Again").await;
    // The header is stdout's and the note stderr's, which reach the shell
    // in either order.
    let shown = last_root(&scripts);
    for line in [
        "ID  VENDOR  RELEASED  ENTRY  NAME\n",
        "demi image models: note: image generation is off for First and Second; turn it on in Settings › Providers\n",
    ] {
        assert_shows(&shown, line);
    }
    assert_eq!(second_images.calls().len(), 1);

    // Settings › Providers still lists each entry's image models, greyed by
    // its switch, and none for an entry that makes no images.
    let catalog = backend.get("/api/models", Some(&master)).await.json::<ModelCatalog>();
    let listed = |id: &str| {
        let provider = catalog.providers.iter().find(|provider| provider.provider_id.as_str() == id);
        serde_json::to_value(&provider.expect("the entry is in the catalog").image_models).unwrap()
    };
    assert_eq!(listed(first.id.as_str()), json!([{ "id": "picture-1", "name": "Picture 1", "released": "2026-01-05" }]));
    assert_eq!(listed(second.as_str()), json!([{ "id": "drawing-1", "name": "Drawing 1", "released": "2026-02-01" }]));
    assert_eq!(listed(tree.as_str()), json!([]));
    backend.close().await;
}

const DRAW: &str = "demi image generate 'a fox' --model picture-1 > /dev/null; echo exit=$?";
const BY_SUBSCRIPTION: &str = "8×8 PNG from picture-1 (Sub, first@example.test)\nexit=0";

/// What a key entry's call says it made.
fn by_key(label: &str) -> String {
    format!("8×8 PNG from picture-1 ({label})\nexit=0")
}

/// A user with two key entries and a subscription entry that make the
/// same vendor's images, in Settings › Providers as Key A, Sub and Key B.
struct Both {
    harness: Harness,
    /// The paired device the agent's shell runs on, kept for the test.
    _device: crate::support::Paired,
    backend: TestBackend,
    master: Session,
    scripts: Arc<Scripts>,
    subscription: Arc<ImageScript>,
    key: Arc<ImageScript>,
    entry: ProviderDto,
    key_a: ProviderDto,
    key_b: ProviderDto,
    /// The subscription's account.
    account: String,
}

async fn both() -> Both {
    both_offering(&PICTURES[..1], &PICTURES[..1]).await
}

/// `both`, its subscription offering `subscription`'s models and its keys
/// `keys`'.
async fn both_offering(subscription: &'static [&'static str], keys: &'static [&'static str]) -> Both {
    let made = demi_agent_store::testing::png(8, 8, 2).into_bytes();
    let subscription = Arc::new(ImageScript::new(subscription, made.clone()));
    let key = Arc::new(ImageScript::new(keys, made).named("Scripted key"));
    let logins = image_family(&subscription);
    let keys = ScriptedKey {
        directory: Arc::new(Directory::default()),
        wires: &[],
        process_host: false,
        images: Some(key.clone()),
    };
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, device, _root, _tree) =
        tree_among(&scripts, Harness::new(), logins.families.clone().with("key", keys)).await;
    let add_key = async |label: &str| {
        let created = backend
            .post(
                "/api/providers",
                Some(&master),
                json!({ "source": "family", "providerType": "key", "label": label, "apiKey": "k" }),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
        created.json::<ProviderAnswer>().provider
    };
    let key_a = add_key("Key A").await;
    logins.login.signs_in("first");
    let entry = device_entry(&backend, &master, &logins).await;
    let named = backend.patch(&format!("/api/providers/{}", entry.id), &master, json!({ "label": "Sub" })).await;
    assert_eq!(named.status, StatusCode::OK, "{}", String::from_utf8_lossy(&named.body));
    let key_b = add_key("Key B").await;
    let account = backend
        .get(&format!("/api/providers/{}/accounts", entry.id), Some(&master))
        .await
        .json::<Accounts>()
        .active
        .unwrap()
        .to_string();
    Both { harness, _device: device, backend, master, scripts, subscription, key, entry, key_a, key_b, account }
}

/// Runs `command` in a turn of the root agent of `FIRST` and answers what
/// the agent was shown.
async fn ran(scripts: &Arc<Scripts>, socket: &mut Socket, turn: &str, command: &str) -> String {
    scripts.root(FIRST, vec![shell(&format!("t-{turn}"), command), say("done")]);
    socket.chat(turn, "Draw").await;
    last_root(scripts)
}

// A few seconds: a login, three entries and a real device's shell jobs of
// nine turns.
//
// Planted defects: a key, which comes first in Settings › Providers, taken
// over the subscription; the keys tried in the order they were added, not
// the one the user dragged; a used-up subscription failing the call
// instead of going on with the next key, or asked again once it is used
// up; a key whose Image Generation is off still paying for the image;
// `--provider` passed over for the first entry, or an entry it names that
// cannot make the model tried anyway or refused without why; `models`
// without the catalog's facts, an alias, the price or an entry's status.
#[tokio::test]
async fn a_call_takes_subscriptions_before_keys_in_the_users_order_hands_on_when_used_up_and_takes_the_entry_named() {
    let Both { harness, _device, backend, master, scripts, subscription, key, entry, key_a, key_b, account } = both().await;
    let patch = async |path: &str, body: serde_json::Value| {
        let answer = backend.patch(path, &master, body).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.body));
    };
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The subscription, though a key comes first in the list; the model's
    // whole page, and the same as JSON.
    let shown = ran(&scripts, &mut socket, "m1", &format!("{DRAW}\ndemi image models picture-1\ndemi image models --json")).await;
    assert_shows(&shown, BY_SUBSCRIPTION);
    assert_shows(
        &shown,
        "ID: picture-1\nVendor: OpenAI\nReleased: 2026-01-05\nEntry: Sub\nName: Picture 1\nAlias of: -\nAliases: picture-latest\n\
         Sizes: -\nCustom size: -\nAspect ratios: 1:1, 16:9\nResolutions: -\nEdits: yes\nMax input images: -\n\
         Transparent background: -\nPrice page: https://example.test/pricing\nPrices:\n\
         \x20 UNIT   MODALITY  AMOUNT  CURRENCY  SIZE  QUALITY  RESOLUTION  DURATION (S)\n\
         \x20 image  -         0.04    USD       -     high     -           -\nEntries:\n\
         \x20 ENTRY  KIND          ACCOUNT             PRICE     STATUS\n\
         \x20 Sub    subscription  first@example.test  included  now\n\
         \x20 Key A  api-key       -                   listed    next\n\
         \x20 Key B  api-key       -                   listed    next\nDescription:\n  Best pictures\n",
    );
    assert_shows(&shown, r#"[{"id":"picture-1","vendor":"OpenAI","vendorId":"openai","name":"Picture 1","#);
    assert_shows(&shown, r#""entries":[{"provider":"Sub","providerId":""#);
    assert_shows(&shown, r#""status":{"status":"now"}},{"provider":"Key A""#);
    assert_shows(&shown, r#""entry":"Sub"}]"#);

    // The user drags Key B above Key A: once the subscription is used up,
    // Key B makes the image at once, and the subscription is not asked
    // again.
    let order = json!({ "providerIds": [key_b.id, entry.id, key_a.id, tree_of(&backend, &master).await] });
    let ordered = backend.put("/api/providers/order", &master, order).await;
    assert_eq!(ordered.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&ordered.body));
    let listed: Vec<String> = backend
        .get("/api/providers", Some(&master))
        .await
        .json::<Providers>()
        .providers
        .into_iter()
        .map(|provider| provider.label)
        .collect();
    assert_eq!(listed, ["Key B", "Sub", "Key A", "Tree"]);
    *subscription.used_up.lock().unwrap() = vec![account.clone()];
    assert_shows(&ran(&scripts, &mut socket, "m2", DRAW).await, &by_key("Key B"));
    let asked = subscription.calls().len();
    let shown = ran(&scripts, &mut socket, "m3", &format!("{DRAW}\ndemi image models")).await;
    assert_shows(&shown, &by_key("Key B"));
    assert_shows(&shown, "ID         VENDOR  RELEASED    ENTRY  NAME\npicture-1  OpenAI  2026-01-05  Key B  Picture 1");
    assert_eq!(subscription.calls().len(), asked);
    let calls: Vec<(Option<String>, Option<String>)> = image_rows(&harness)
        .into_iter()
        .skip(1)
        .map(|row| (row.account_id, row.outcome))
        .collect();
    assert_eq!(
        calls,
        [
            (Some(account.clone()), Some("usage_limit".to_owned())),
            (None, Some("answered".to_owned())),
            (None, Some("answered".to_owned())),
        ]
    );

    // The entry the agent names, and why one cannot.
    let shown = ran(
        &scripts,
        &mut socket,
        "m4",
        &[
            "demi image generate 'a fox' --model picture-1 --provider 'Key A' > /dev/null; echo exit=$?",
            "demi image generate 'a fox' --model picture-1 --provider Sub; echo exit=$?",
            "demi image generate 'a fox' --model picture-1 --provider Tree; echo exit=$?",
            "demi image generate 'a fox' --model picture-1 --provider Nope; echo exit=$?",
        ]
        .join("\n"),
    )
    .await;
    assert_shows(&shown, &by_key("Key A"));
    assert_shows(
        &shown,
        "demi image generate: Sub cannot make \"picture-1\": its usage limit is reached, resetting at Sep 24, 8:30 AM; Key B and Key A can\nexit=1",
    );
    assert_shows(&shown, "demi image generate: Tree cannot make \"picture-1\": it does not make \"picture-1\"; Key B and Key A can\nexit=1");
    assert_shows(&shown, "demi image generate: no provider entry is named or has the id \"Nope\"; `demi providers list` lists them\nexit=1");
    assert_eq!(subscription.calls().len(), asked);

    // Key B's Image Generation off: Key A makes it; both off, the call
    // fails with the subscription's limit instead of paying for the image.
    patch(&format!("/api/providers/{}", key_b.id), json!({ "media": { "image": { "enabled": false } } })).await;
    let shown = ran(&scripts, &mut socket, "m5", &format!("{DRAW}\ndemi image models picture-1")).await;
    assert_shows(&shown, &by_key("Key A"));
    assert_shows(&shown, "\x20 Key B  api-key       -                   listed    Image Generation off\n");
    patch(&format!("/api/providers/{}", key_a.id), json!({ "media": { "image": { "enabled": false } } })).await;
    let shown = ran(&scripts, &mut socket, "m6", DRAW).await;
    assert_shows(&shown, "demi image generate: Scripted's usage limit is reached");
    assert_shows(&shown, "\nexit=1");
    assert_eq!(key.calls().len(), 4);
    backend.close().await;
}

/// The id of the tree's own entry, which makes no images.
async fn tree_of(backend: &TestBackend, master: &Session) -> String {
    backend
        .get("/api/providers", Some(master))
        .await
        .json::<Providers>()
        .providers
        .into_iter()
        .find(|provider| provider.label == "Tree")
        .unwrap()
        .id
        .to_string()
}

// About a second.
//
// Planted defect: the entries read once before the first request, so a key
// turned off while the subscription's request was under way still pays for
// the image.
#[tokio::test]
async fn a_key_turned_off_while_the_subscriptions_request_is_under_way_is_not_sent_to() {
    let Both { harness: _harness, _device, backend, master, scripts, subscription, key, key_a, key_b, account, .. } = both().await;
    *subscription.used_up.lock().unwrap() = vec![account];
    let (release, gate) = tokio::sync::oneshot::channel();
    *subscription.gate.lock().unwrap() = Some(gate);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let mut counts = subscription.counts.subscribe();
    scripts.root(FIRST, vec![shell("t1", DRAW), say("done")]);
    socket.send(&crate::conversations::send("m1", "Draw")).await;
    // The subscription's request is under way: the keys go off meanwhile.
    counts.wait_for(|(asked, _)| *asked == 1).await.unwrap();
    for id in [&key_a.id, &key_b.id] {
        let off = backend.patch(&format!("/api/providers/{id}"), &master, json!({ "media": { "image": { "enabled": false } } })).await;
        assert_eq!(off.status, StatusCode::OK, "{}", String::from_utf8_lossy(&off.body));
    }
    release.send(()).unwrap();
    socket.until_idle().await;
    let shown = last_root(&scripts);
    assert_shows(&shown, "demi image generate: Scripted's usage limit is reached");
    assert_shows(&shown, "\nexit=1");
    assert!(key.calls().is_empty());
    backend.close().await;
}

// About a second.
//
// Planted defect: a refusal read as an entry that cannot make the image,
// so the next entry, a key that pays for it, is asked the same prompt.
#[tokio::test]
async fn a_refusal_on_one_entry_never_calls_another() {
    let Both { harness: _harness, _device, backend, master, scripts, subscription, key, .. } = both().await;
    *subscription.refusal.lock().unwrap() = Some("Your request was rejected by the safety system.".to_owned());
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let shown = ran(&scripts, &mut socket, "m1", DRAW).await;
    assert_shows(&shown, "demi image generate: Scripted refused the request: Your request was rejected by the safety system.\nexit=1");
    assert_eq!(subscription.calls().len(), 1);
    assert!(key.calls().is_empty());
    backend.close().await;
}

// About a second: one scripted server, a real device's shell jobs of two
// turns.
//
// Planted defects: an image model added to an entry with no image shape;
// a compatible entry's image models in the model menu; its images sent
// anywhere but its own endpoint, without its key, or in another shape
// than OpenAI's.
#[tokio::test]
async fn a_compatible_api_with_the_openai_images_shape_makes_the_images_of_the_models_its_user_added() {
    let vendor = MockVendor::start().await;
    let image = demi_agent_store::testing::png(8, 8, 3).into_bytes();
    vendor.respond_at(
        "/v1/images/generations",
        MockResponse::status(200)
            .header("content-type", "application/json")
            .chunk(json!({ "created": 1_790_000_000, "data": [{ "b64_json": B64Bytes::from(image.clone()) }] }).to_string()),
    );
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _device, _root, _tree) =
        tree_among(&scripts, Harness::new(), demi_backend::families::builtin()).await;
    let created = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({ "source": "compatible", "providerType": "openai", "wireApi": "chat-completions", "label": "Proxy",
                    "apiKey": "sk-proxy", "baseUrl": vendor.url("/v1") }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let proxy = created.json::<ProviderAnswer>().provider;
    assert_eq!((proxy.media.image.shape, proxy.media.image.enabled), (None, true));
    let path = format!("/api/providers/{}", proxy.id);
    let models = json!({ "models": [{ "kind": "image", "id": "gpt-image-2", "displayName": "GPT Image 2", "contextWindow": null,
        "outputLimit": null, "thinkingEfforts": [], "acceptedExtensions": null, "fastTier": null }] });
    let unmade = backend.patch(&path, &master, models.clone()).await;
    assert_eq!(unmade.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
    let shaped = backend.patch(&path, &master, json!({ "media": { "image": { "shape": "openai-images" } } })).await;
    assert_eq!(shaped.status, StatusCode::OK, "{}", String::from_utf8_lossy(&shaped.body));
    assert_eq!(shaped.json::<ProviderAnswer>().provider.media.image.shape.map(|shape| shape.to_string()).as_deref(), Some("openai-images"));
    let added = backend.patch(&path, &master, models).await;
    assert_eq!(added.status, StatusCode::OK, "{}", String::from_utf8_lossy(&added.body));
    // Its image model is no model of the menu, so the menu has no group
    // for it.
    let catalog = backend.get("/api/models", Some(&master)).await.json::<ModelCatalog>();
    let menu = catalog.providers.iter().find(|provider| provider.provider_id == proxy.id).unwrap();
    assert!(menu.models.is_empty(), "{:?}", menu.models);

    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let shown = ran(
        &scripts,
        &mut socket,
        "m1",
        "demi image models\ndemi image generate 'a fox' --model gpt-image-2 --aspect 16:9 > fox.png; echo exit=$?",
    )
    .await;
    assert_shows(&shown, "ID           VENDOR  RELEASED  ENTRY  NAME\ngpt-image-2  -       -         Proxy  GPT Image 2\n");
    assert_shows(&shown, "8×8 PNG from gpt-image-2 (Proxy)\n");
    let sent = vendor.requests();
    let sent = sent.last().unwrap();
    assert_eq!((sent.uri.path(), sent.header("authorization")), ("/v1/images/generations", Some("Bearer sk-proxy")));
    assert_eq!(
        sent.json(),
        json!({ "prompt": "a fox", "model": "gpt-image-2", "size": "1536x864", "background": "opaque",
                "output_format": "png", "quality": "auto" })
    );

    // None keeps the image model, unused and hidden: a text model's edit,
    // which sends the whole list, still goes through, a new image model
    // does not, and the shape brings the kept one back.
    let none = backend.patch(&path, &master, json!({ "media": { "image": { "shape": null } } })).await;
    assert_eq!(none.status, StatusCode::OK, "{}", String::from_utf8_lossy(&none.body));
    let image_model = |id: &str| json!({ "kind": "image", "id": id, "displayName": id, "contextWindow": null,
        "outputLimit": null, "thinkingEfforts": [], "acceptedExtensions": null, "fastTier": null });
    let text_model = json!({ "kind": "text", "id": "qwen", "displayName": "Qwen", "contextWindow": 32768,
        "outputLimit": null, "thinkingEfforts": [], "acceptedExtensions": [], "fastTier": null });
    let edited = backend
        .patch(&path, &master, json!({ "models": [image_model("gpt-image-2"), text_model.clone()] }))
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", String::from_utf8_lossy(&edited.body));
    let added = backend
        .patch(&path, &master, json!({ "models": [image_model("gpt-image-2"), image_model("other"), text_model] }))
        .await;
    assert_eq!(added.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
    let catalog = backend.get("/api/models", Some(&master)).await.json::<ModelCatalog>();
    let hidden = catalog.providers.iter().find(|provider| provider.provider_id == proxy.id).unwrap();
    assert!(hidden.image_models.is_empty(), "{:?}", hidden.image_models);
    let back = backend.patch(&path, &master, json!({ "media": { "image": { "shape": "openai-images" } } })).await;
    assert_eq!(back.status, StatusCode::OK, "{}", String::from_utf8_lossy(&back.body));
    let catalog = backend.get("/api/models", Some(&master)).await.json::<ModelCatalog>();
    let shown = catalog.providers.iter().find(|provider| provider.provider_id == proxy.id).unwrap();
    assert_eq!(shown.image_models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["gpt-image-2"]);
    backend.close().await;
}

// A model and its alias are one model: a used-up subscription that lists
// both names and keys that list only the alias, as Codex lists
// gpt-image-2 beside its dated name and a proxy only the short one. The
// model's row has every entry that offers either name, and a call by
// either goes on to a key, which is sent the name it lists. A few seconds:
// a login, three entries and three turns.
//
// Planted defect: a row or a call that counts only the entries listing
// the very name asked, so the keys go unseen and the call fails used up.
#[tokio::test(flavor = "multi_thread")]
async fn a_model_and_its_alias_are_one_model_offered_by_every_entry_that_lists_either() {
    let Both { harness: _harness, _device, scripts, subscription, key, backend, master, account, .. } =
        both_offering(&["picture-1", "picture-latest"], &["picture-latest"]).await;
    *subscription.used_up.lock().unwrap() = vec![account.clone()];
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    let shown = ran(
        &scripts,
        &mut socket,
        "m1",
        "demi image generate 'a fox' --model picture-1 > /dev/null; echo exit=$?",
    )
    .await;
    assert_shows(&shown, "exit=0");
    let sent: Vec<String> = key.calls().into_iter().map(|(_, request)| request.model).collect();
    assert_eq!(sent, ["picture-latest"]);

    let shown = ran(&scripts, &mut socket, "m2", "demi image models\ndemi image models picture-latest").await;
    assert_shows(&shown, "ID         VENDOR  RELEASED    ENTRY  NAME\npicture-1  OpenAI  2026-01-05  Key A  Picture 1\n");
    assert_shows(&shown, "ID: picture-1\n");
    assert_shows(&shown, "\x20 Key A  api-key       -                   listed    now\n");
    assert_shows(&shown, "\x20 Key B  api-key       -                   listed    next\n");
    backend.close().await;
}

// A media catalog downloaded later describes the users' media models anew:
// it reaches each page's state of the providers, which then reads them
// again, rather than waiting for an unrelated change of an entry. Tens of
// milliseconds.
//
// Planted defect: the catalog's change announced to nobody, so the page
// shows the old catalog's models until an entry changes.
#[tokio::test]
async fn a_newer_media_catalog_reaches_the_pages_state_of_the_providers() {
    let server = MockVendor::start().await;
    server.respond_at("/media.json", MockResponse::status(503));
    let (backend, master) = Harness::new()
        .with_media_catalog(server.url("/media.json").to_string())
        .start_set_up()
        .await;
    let created = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({ "source": "compatible", "providerType": "openai", "wireApi": "chat-completions", "label": "Proxy",
                    "baseUrl": "https://proxy.example/v1" }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let mut channel = backend.sync(&master).await;
    let state = channel.snapshot().await;
    assert_eq!(state.providers[0].provider.media.catalog.to_string(), "2026-09-01T00:00:00.000Z");

    let newer = crate::support::MEDIA_CATALOG.replace("2026-09-01T00:00:00Z", "2026-10-01T00:00:00Z");
    server.respond_at("/media.json", MockResponse::status(200).header("content-type", "application/json").chunk(newer));
    backend.refresh_media_catalog().await;
    let received = channel
        .until(|event| matches!(event, SyncEvent::Providers { providers } if providers[0].provider.media.catalog.to_string() != "2026-09-01T00:00:00.000Z"))
        .await;
    let Some(SyncEvent::Providers { providers }) = received.into_iter().last() else {
        unreachable!("until ends at a providers event");
    };
    assert_eq!(providers[0].provider.media.catalog.to_string(), "2026-10-01T00:00:00.000Z");
    backend.close().await;
}
