//! The agent's `demi image` (`images.md`), the `image` plugin over the
//! port's Providers service: an image made to a file and to `demi file
//! view`, an edit of the Host's files, warnings for what the result did not
//! honour, a used-up account under Balanced moving the call to another, a
//! usage ledger row per call, a stopped call's cancelled row, no account
//! named to a shared instance's user, an entry whose Image Generation is
//! off never chosen, and the entry a vendor's model is made through:
//! Automatic's subscription before a key, the user's choice while it can,
//! and the key once the subscription is used up. The model
//! is the scripted family of the subagent scenarios, the image role a
//! scripted family's, and the device a real runner. No test calls a real
//! model.

use std::sync::Arc;

use demi_provider_common::images::ImageVendor;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::providers::{Accounts, LoginState, ModelCatalog, ProviderAnswer, ProviderDto};
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::json;

use crate::accounts::{awaited, device_entry, ended, start_login};
use crate::agent_profiles::{assert_shows, last_root};
use crate::conversations::{FIRST, SECOND, Socket, choose, create, working_on};
use demi_web_api_protocol::auth::Role;
use crate::families::{Directory, ImageScript, KEY, LoginScript, QuotaScript, ScriptedKey, ScriptedSubscription};
use crate::subagents::{Scripts, say, shell, tree_among};
use crate::support::{Harness, Session, TestBackend};
use crate::usage::{Row, rows};

/// The scripted family's image models, the older first, so the list shows
/// the newer first; the test backend's catalog says what each is
/// (`support::MEDIA_CATALOG`).
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
    let images = Arc::new(ImageScript::new(PICTURES, made.clone()));
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
    let said = "700×700 PNG from picture-1 (Scripted, first@example.test)\n";
    for shown in [
        &format!("{said}same\n"),
        &format!("[image 1: image/png, 700 × 700 px, {} bytes]", made.len()),
        &format!(
            "{said}demi image generate: warning: asked for 16:9, got 700×700\ndemi image generate: warning: asked for a transparent background, got an opaque image\n"
        ),
        &format!("{said}exit=0\n"),
        "demi image edit: notes.txt: not a PNG, JPEG or WebP image\nexit=1\n",
        "demi image generate: no provider makes \"nope\"; demi image models lists them\nexit=1\n",
        "700×700 PNG from picture-2 (Scripted, first@example.test)\n",
        &format!(
            "OpenAI, through {label} (first@example.test)\n  picture-2  2026-06-01  Faster pictures; takes 1:1\n  picture-1  2026-01-05  Best pictures; takes 1:1 or 16:9; edits",
            label = entry.label
        ),
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
    assert_shows(&last_root(&scripts), "700×700 PNG from picture-1 (Scripted, first@example.test)\nexit=0");
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
    assert_shows(&asked, "700×700 PNG from picture-1 (Scripted)");
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
    assert_shows(&last_root(&scripts), "8×8 PNG from picture-1 (Scripted, second@example.test)\nexit=0");
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
// Planted defects: a switch stored for an entry that makes no images; an
// entry whose Image Generation is off still chosen or listed by `demi
// image models`; a --model of an entry that is off told as a model nobody
// makes; every entry off told as none signed in.
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
    // An entry that makes no images has no switch to set.
    let refused = backend
        .patch(&format!("/api/providers/{tree}"), &master, json!({ "imageGeneration": false }))
        .await;
    assert_eq!(refused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
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
    let off = set(first.id.as_str(), json!({ "imageGeneration": false })).await;
    assert!(!off.image_generation);
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
        "8×8 PNG from drawing-1 (Scripted, first@example.test)\nOpenAI, through Second (first@example.test)\n  drawing-1  2026-02-01  Line drawings; takes 1:1\n",
        "demi image generate: \"picture-1\" is First's, whose image generation is off in Settings › Providers\nexit=1",
    ] {
        assert_shows(&asked, shown);
    }
    assert!(first_images.calls().is_empty());

    // Both off: no entry makes images, and the call names them.
    set(second.as_str(), json!({ "imageGeneration": false })).await;
    scripts.root(
        FIRST,
        vec![shell("t2", "demi image models; echo exit=$?"), say("none")],
    );
    socket.chat("m2", "Again").await;
    assert_shows(
        &last_root(&scripts),
        "demi image models: image generation is off for First and Second; turn it on in Settings › Providers\nexit=1",
    );
    assert_eq!(second_images.calls().len(), 1);
    backend.close().await;
}

const DRAW: &str = "demi image generate 'a fox' --model picture-1 > /dev/null; echo exit=$?";
const BY_SUBSCRIPTION: &str = "8×8 PNG from picture-1 (Scripted, first@example.test)\nexit=0";
const BY_KEY: &str = "8×8 PNG from picture-1 (Scripted key)\nexit=0";

/// A user with a subscription entry and a key entry that make the same
/// vendor's images, the key first in Settings › Providers.
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
    key_entry: ProviderDto,
    /// The subscription's account.
    account: String,
}

async fn both() -> Both {
    let made = demi_agent_store::testing::png(8, 8, 2).into_bytes();
    let subscription = Arc::new(ImageScript::new(&PICTURES[..1], made.clone()));
    let key = Arc::new(ImageScript::new(&PICTURES[..1], made).named("Scripted key"));
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
    let created = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({ "source": "family", "providerType": "key", "label": "Key", "apiKey": "k" }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let key_entry = created.json::<ProviderAnswer>().provider;
    logins.login.signs_in("first");
    let entry = device_entry(&backend, &master, &logins).await;
    let account = backend
        .get(&format!("/api/providers/{}/accounts", entry.id), Some(&master))
        .await
        .json::<Accounts>()
        .active
        .unwrap()
        .to_string();
    Both { harness, _device: device, backend, master, scripts, subscription, key, entry, key_entry, account }
}

/// The entry each vendor's Automatic takes, as `GET /api/models` answers.
async fn automatic(backend: &TestBackend, master: &Session) -> Vec<(ImageVendor, Option<String>)> {
    let catalog = backend.get("/api/models", Some(master)).await.json::<ModelCatalog>();
    catalog.image_entries.into_iter().map(|(vendor, entry)| (vendor, entry.map(|id| id.to_string()))).collect()
}

/// Runs `command` in a turn of the root agent of `FIRST` and answers what
/// the agent was shown.
async fn ran(scripts: &Arc<Scripts>, socket: &mut Socket, turn: &str, command: &str) -> String {
    scripts.root(FIRST, vec![shell(&format!("t-{turn}"), command), say("done")]);
    socket.chat(turn, "Draw").await;
    last_root(scripts)
}

// A few seconds: a login, two entries and a real device's shell jobs of
// seven turns.
//
// Planted defects: Automatic taking the key, which comes first in Settings
// › Providers, over the subscription; the chosen entry kept while its
// Image Generation is off; a used-up subscription failing the call instead
// of going on with the key, or asked again once it is used up; a key whose
// Image Generation is off still paying for the image.
#[tokio::test]
async fn automatic_takes_the_subscription_before_a_key_a_chosen_entry_while_it_can_and_the_key_once_the_subscription_is_used_up() {
    let Both { harness, _device, backend, master, scripts, subscription, key, entry, key_entry, account } = both().await;
    let patch = async |path: &str, body: serde_json::Value| {
        let answer = backend.patch(path, &master, body).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.body));
    };
    let key_path = format!("/api/providers/{}", key_entry.id);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let automatic = async || automatic(&backend, &master).await;
    // Automatic: the subscription, though the key comes first in the list,
    // as Settings › Agent shows it beside Automatic.
    assert_eq!(automatic().await, [(ImageVendor::OpenAi, Some(entry.id.to_string()))]);
    let shown = ran(&scripts, &mut socket, "m1", &format!("{DRAW}\ndemi image models")).await;
    assert_shows(&shown, BY_SUBSCRIPTION);
    assert_shows(
        &shown,
        &format!("OpenAI, through {} (first@example.test)\n  picture-1  2026-01-05  Best pictures; takes 1:1 or 16:9; edits", entry.label),
    );

    // The user's choice, while it can make the model.
    patch("/api/settings/preferences", json!({ "imageEntries": { "openai": key_entry.id } })).await;
    let shown = ran(&scripts, &mut socket, "m2", &format!("{DRAW}\ndemi image models")).await;
    assert_shows(&shown, BY_KEY);
    assert_shows(&shown, "OpenAI, through Key\n");
    patch(&key_path, json!({ "imageGeneration": false })).await;
    assert_shows(&ran(&scripts, &mut socket, "m3", DRAW).await, BY_SUBSCRIPTION);

    // Automatic again: once the subscription is used up, the key makes the
    // image at once, and the used-up subscription is not asked again.
    patch(&key_path, json!({ "imageGeneration": true })).await;
    patch("/api/settings/preferences", json!({ "imageEntries": {} })).await;
    *subscription.used_up.lock().unwrap() = vec![account.clone()];
    assert_shows(&ran(&scripts, &mut socket, "m4", DRAW).await, BY_KEY);
    assert_eq!(automatic().await, [(ImageVendor::OpenAi, Some(key_entry.id.to_string()))]);
    let asked = subscription.calls().len();
    assert_shows(&ran(&scripts, &mut socket, "m5", DRAW).await, BY_KEY);
    assert_eq!(subscription.calls().len(), asked);
    let calls: Vec<(Option<String>, Option<String>)> = image_rows(&harness)
        .into_iter()
        .skip(3)
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
    assert_eq!(key.calls().iter().map(|(account, _)| account.as_str()).collect::<Vec<_>>(), [KEY, KEY, KEY]);

    // With the key's Image Generation off, the call fails with the limit
    // instead of paying for the image.
    patch(&key_path, json!({ "imageGeneration": false })).await;
    assert_eq!(automatic().await, [(ImageVendor::OpenAi, None)]);
    let shown = ran(&scripts, &mut socket, "m6", DRAW).await;
    assert_shows(&shown, "demi image generate: Scripted's usage limit is reached");
    assert_shows(&shown, "\nexit=1");
    backend.close().await;
}

// About a second.
//
// Planted defect: the entries read once before the first request, so a key
// turned off while the subscription's request was under way still pays for
// the image.
#[tokio::test]
async fn a_key_turned_off_while_the_subscriptions_request_is_under_way_is_not_sent_to() {
    let Both { harness: _harness, _device, backend, master, scripts, subscription, key, key_entry, account, .. } = both().await;
    *subscription.used_up.lock().unwrap() = vec![account];
    let (release, gate) = tokio::sync::oneshot::channel();
    *subscription.gate.lock().unwrap() = Some(gate);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let mut counts = subscription.counts.subscribe();
    scripts.root(FIRST, vec![shell("t1", DRAW), say("done")]);
    socket.send(&crate::conversations::send("m1", "Draw")).await;
    // The subscription's request is under way: the key goes off meanwhile.
    counts.wait_for(|(asked, _)| *asked == 1).await.unwrap();
    let off = backend.patch(&format!("/api/providers/{}", key_entry.id), &master, json!({ "imageGeneration": false })).await;
    assert_eq!(off.status, StatusCode::OK, "{}", String::from_utf8_lossy(&off.body));
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

// About a second.
//
// Planted defect: an account marked used up without telling the pages, so
// Settings › Agent keeps naming the subscription beside Automatic.
#[tokio::test]
async fn a_subscription_used_up_without_quota_headers_tells_the_pages_and_automatic_moves_to_the_key() {
    let Both { harness: _harness, _device, backend, master, scripts, subscription, entry, key_entry, account, .. } = both().await;
    assert_eq!(automatic(&backend, &master).await, [(ImageVendor::OpenAi, Some(entry.id.to_string()))]);
    let mut page = backend.sync(&master).await;
    page.snapshot().await;
    *subscription.used_up.lock().unwrap() = vec![account];
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    assert_shows(&ran(&scripts, &mut socket, "m1", DRAW).await, BY_KEY);
    // The page learns of it as it does of any change of the providers,
    // and the catalog it then reads names the key.
    let told = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        page.until(|event| matches!(event, SyncEvent::Providers { .. })),
    )
    .await;
    assert!(told.is_ok(), "the pages were not told");
    assert_eq!(automatic(&backend, &master).await, [(ImageVendor::OpenAi, Some(key_entry.id.to_string()))]);
    backend.close().await;
}
