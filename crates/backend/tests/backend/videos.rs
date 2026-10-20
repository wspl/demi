//! The agent's `demi video`, the `video` plugin over the port's Providers
//! service.
//!
//! Scenarios:
//!
//! - A video's job is asked for, its states told and its MP4 downloaded to a
//!   file, with a usage ledger row of its seconds and cost.
//! - A job the vendor failed or that expired is told in the design's words.
//! - A used-up account is handed on before the ask, never after it.
//! - A job still running at the bound.
//! - A stopped call's cancelled row, with the job cancelled at a vendor that
//!   can.
//! - A compatible API whose video shape and models the agent sets with `demi
//!   providers`, its OpenRouter Videos and OpenAI Videos requests sent to a
//!   scripted server.
//!
//! The subscription's video role is a scripted family's, the device a real
//! runner. No test calls a real model.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use demi_backend_user_shard::tuning::MediaTuning;
use demi_provider_common::media::MediaFailure;
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_provider_common::videos::JobState;
use demi_web_api_protocol::providers::{Accounts, LoginState, ProviderAnswer};
use reqwest::StatusCode;
use serde_json::json;

use crate::accounts::{awaited, device_entry, ended, start_login};
use crate::agent_profiles::{assert_shows, last_root};
use crate::conversations::{FIRST, Socket};
use crate::families::{Directory, LoginScript, QuotaScript, ScriptedSubscription, VideoScript};
use crate::permissions::{decide, permissions, settled};
use crate::subagents::{Scripts, background, say, shell, tree_among};
use crate::support::Harness;
use crate::usage::{Row, rows};

/// One second of 64×36 px with a sound track, the plugin's fixture.
const CLIP: &[u8] = include_bytes!("../../../plugin-video/plugin/tests/plugin-video/fixtures/clip.mp4");

/// A `device` family whose accounts make the videos `videos` scripts, and
/// the scripts of its logins.
fn video_family(videos: &Arc<VideoScript>) -> crate::accounts::Scripts {
    let login = Arc::new(LoginScript::default());
    let quota = Arc::new(QuotaScript::free(Vec::new()));
    let family = ScriptedSubscription {
        login: login.clone(),
        quota: quota.clone(),
        directory: Arc::new(Directory::default()),
        runs: None,
        process_host: false,
        images: None,
        videos: Some(videos.clone()),
    };
    crate::accounts::Scripts {
        login,
        quota,
        families: demi_backend::families::builtin().with("device", family),
    }
}

/// A harness whose jobs are read every 10 ms within `bound`, so a scenario
/// sees each state at once.
pub(crate) fn reading_within(bound: Duration) -> Harness {
    let mut harness = Harness::new();
    harness.media = MediaTuning { video_reads_every: Duration::from_millis(10), video_bound: bound };
    harness
}

/// The ledger's video rows.
fn video_rows(harness: &Harness) -> Vec<Row> {
    rows(harness).into_iter().filter(|row| row.purpose.as_deref() == Some("video")).collect()
}

async fn ran(scripts: &Arc<Scripts>, socket: &mut Socket, turn: &str, command: &str) -> String {
    scripts.root(FIRST, vec![shell(&format!("t-{turn}"), command), say("done")]);
    socket.chat(turn, "Film").await;
    last_root(scripts)
}

// About two seconds: three logins and a real device's shell jobs of eight
// turns, one woken by the user's decision.
//
// Planted defects:
//
// - The job named only at its end.
// - A state read but not told.
// - The ledger's seconds or cost left out.
// - A used-up account at the ask failing the call instead of moving on.
// - A used-up account during the job handed on, which asks and pays for a
//   second job.
// - A stopped call that writes no row.
// - `--video on` refused for a vendor's entry, whose shape is fixed.
#[tokio::test]
async fn a_subscription_makes_a_video_told_as_its_job_moves_handing_on_only_before_the_ask() {
    let videos = Arc::new(VideoScript::new(&["clip-1"], Bytes::from_static(CLIP)));
    let logins = video_family(&videos);
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _device, root, _tree) =
        tree_among(&scripts, reading_within(Duration::from_secs(60)), logins.families.clone()).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // Before any sign-in, nothing makes videos.
    let shown = ran(&scripts, &mut socket, "m1", "demi video generate 'a boat' --model clip-1; echo exit=$?").await;
    assert_shows(
        &shown,
        "demi video generate: no provider makes videos; sign in to Grok, or add an xAI, Google, MiniMax, BytePlus ModelArk or Volcengine Ark key or a compatible API with a video shape, in Settings › Providers\nexit=1",
    );

    logins.login.signs_in("first");
    let entry = device_entry(&backend, &master, &logins).await;
    let named = backend.patch(&format!("/api/providers/{}", entry.id), &master, json!({ "label": "Sub" })).await;
    assert_eq!(named.status, StatusCode::OK, "{}", String::from_utf8_lossy(&named.body));
    assert_eq!(entry.media.video.shape.map(|shape| shape.to_string()).as_deref(), Some("grok"));
    std::fs::write(format!("{root}/expected.mp4"), CLIP).unwrap();
    videos.answer([Ok(JobState::Queued), Ok(JobState::Running { progress: Some(40) }), Ok(VideoScript::done())]);
    let shown = ran(
        &scripts,
        &mut socket,
        "m2",
        "demi video generate 'a paper boat' --model clip-1 --duration 1 --resolution 480p --seed 7 > boat.mp4 && cmp boat.mp4 expected.mp4 && echo same\ndemi video models",
    )
    .await;
    assert_shows(
        &shown,
        "demi video generate: asked clip-1 (Sub, first@example.test), job job-1\n\
         demi video generate: queued\n\
         demi video generate: running, 40%\n\
         1 s 64×36 MP4 with sound from clip-1 (Sub, first@example.test), $0.05\n\
         demi video generate: warning: Grok takes no --seed\n\
         demi video generate: warning: asked for 480p, got 64×36\n\
         same\n",
    );
    assert_shows(&shown, "ID      VENDOR  RELEASED    ENTRY  NAME\nclip-1  xAI     2026-03-01  Sub    Clip 1");
    let first = backend
        .get(&format!("/api/providers/{}/accounts", entry.id), Some(&master))
        .await
        .json::<Accounts>()
        .active
        .unwrap()
        .to_string();
    let row = video_rows(&harness).pop().unwrap();
    assert_eq!(
        (row.turn_id.as_deref(), row.account_id.as_deref(), row.model_id.as_str(), row.vendor_id.as_deref()),
        (Some("m2"), Some(first.as_str()), "clip-1", Some("job-1"))
    );
    assert_eq!(
        (row.outcome.as_deref(), row.media_seconds, row.cost, row.cost_currency.as_deref()),
        (Some("answered"), Some(1.0), Some(0.05), Some("usd"))
    );
    let asked = &videos.asks()[0].1;
    assert_eq!((asked.prompt.as_str(), asked.duration, asked.seed), ("a paper boat", Some(1), Some(7)));

    // A job the vendor fails, and one that expires there, are told in the
    // design's words; each is a failed row.
    videos.answer([
        Ok(JobState::Failed { message: "Video generation failed".into(), trace: Default::default() }),
        Ok(JobState::Expired),
    ]);
    let shown = ran(
        &scripts,
        &mut socket,
        "m3",
        "demi video generate 'x' --model clip-1 > a.mp4; echo exit=$?\ndemi video generate 'x' --model clip-1 > b.mp4; echo exit=$?",
    )
    .await;
    assert_shows(&shown, "demi video generate: Grok failed the video: Video generation failed\nexit=1\n");
    assert_shows(&shown, "demi video generate: the job expired at Grok before it ran\nexit=1");
    let outcomes: Vec<Option<String>> = video_rows(&harness).into_iter().skip(1).map(|row| row.outcome).collect();
    assert_eq!(outcomes, [Some("failed".to_owned()), Some("failed".to_owned())]);

    // A second account under Balanced: the first, used up at the ask, hands
    // the call on before any job is asked of it.
    let path = format!("/api/providers/{}/accounts", entry.id);
    logins.login.signs_in("second");
    let login = start_login(&backend, &master, &format!("{path}/login"), json!({})).await;
    let LoginState::Completed { credential_id: second, .. } = awaited(&backend, &master, &login, ended).await else {
        panic!("the second login did not complete");
    };
    let rotated = backend.patch(&format!("/api/providers/{}", entry.id), &master, json!({ "rotation": "balanced" })).await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", String::from_utf8_lossy(&rotated.body));
    *videos.used_up.lock().unwrap() = vec![first.clone()];
    videos.answer([Ok(VideoScript::done())]);
    let shown = ran(&scripts, &mut socket, "m4", "demi video generate 'x' --model clip-1 > c.mp4; echo exit=$?").await;
    assert_shows(&shown, "1 s 64×36 MP4 with sound from clip-1 (Sub, second@example.test), $0.05\nexit=0");
    let accounts: Vec<(Option<String>, Option<String>)> =
        video_rows(&harness).into_iter().skip(3).map(|row| (row.account_id, row.outcome)).collect();
    assert_eq!(
        accounts,
        [(Some(first.clone()), Some("usage_limit".to_owned())), (Some(second.to_string()), Some("answered".to_owned()))]
    );

    // Once the job is asked for, a limit the job meets fails the call: no
    // second job is asked of the other account.
    videos.used_up.lock().unwrap().clear();
    let asks = videos.asks().len();
    videos.answer([Err(MediaFailure::UsageLimit { retry_at: None, trace: Default::default() })]);
    let shown = ran(&scripts, &mut socket, "m5", "demi video generate 'x' --model clip-1 > d.mp4; echo exit=$?").await;
    assert_shows(&shown, "demi video generate: Grok's usage limit is reached\nexit=1");
    assert_eq!(videos.asks().len(), asks + 1);

    // A stopped call stops waiting and leaves a cancelled row; Grok has no
    // cancellation, so the job runs on at the vendor.
    let mut counts = videos.counts.subscribe();
    let asked = counts.borrow().0;
    scripts.root(
        FIRST,
        vec![
            shell(
                "t6",
                "demi video generate 'a boat' --model clip-1 > held.mp4 & call=$!; until [ -f go ]; do sleep 0.05; done; kill $call; wait $call; echo stopped",
            ),
            say("stopped"),
        ],
    );
    socket.send(&crate::conversations::send("m6", "Film and stop")).await;
    counts.wait_for(|(now, read)| *now > asked && *read > 0).await.unwrap();
    std::fs::write(format!("{root}/go"), "").unwrap();
    socket.until_idle().await;
    crate::support::eventually("the stopped call's row is written", async || {
        video_rows(&harness).last().and_then(|row| row.outcome.clone()).as_deref() == Some("cancelled")
    })
    .await;
    assert_eq!(video_rows(&harness).last().unwrap().turn_id.as_deref(), Some("m6"));

    // The vendor's entry turns its videos off and on again, in its fixed
    // shape, once the user allows managing providers: the first change
    // waits for the answer, and the rest runs under the grant.
    scripts.root(
        FIRST,
        vec![
            background(
                "t7",
                "demi providers edit Sub --video off > /dev/null\n\
                 demi providers show Sub | grep '^Videos'\n\
                 demi providers edit Sub --video on > /dev/null; echo exit=$?\n\
                 demi providers show Sub | grep '^Videos'",
            ),
            say("asked"),
            say("switched"),
            say("switched"),
        ],
    );
    socket.chat("m7", "Film").await;
    assert_shows(
        &last_root(&scripts),
        "waiting for the user's permission to manage providers; its end will report to you, so do not run it again",
    );
    let requested = permissions(&backend, &master, FIRST).await;
    let allowed = decide(&backend, &master, FIRST, requested.requests[0].id().as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&allowed.body));
    let shown = settled(
        &scripts,
        &backend,
        &master,
        FIRST,
        &["The user allowed this conversation to manage providers.", "(t7) ended with exit code 0"],
    )
    .await;
    assert_shows(&shown, "Videos: grok, Video Generation off\nexit=0\nVideos: grok, Video Generation on");
    backend.close().await;
}

// About a second: a login and a real device's shell jobs of two turns,
// whose jobs are read until their bound of 300 ms passes.
//
// Planted defects: no bound, so the call waits for a job that never ends;
// the bound counted from the ask's answer, so an ask that took the whole
// bound is still waited for.
#[tokio::test]
async fn a_job_still_running_at_the_bound_fails_naming_its_id() {
    let videos = Arc::new(VideoScript::new(&["clip-1"], Bytes::from_static(CLIP)));
    let logins = video_family(&videos);
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _device, _root, _tree) =
        tree_among(&scripts, reading_within(Duration::from_millis(300)), logins.families.clone()).await;
    logins.login.signs_in("first");
    device_entry(&backend, &master, &logins).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    let shown = ran(&scripts, &mut socket, "m1", "demi video generate 'x' --model clip-1 > a.mp4; echo exit=$?").await;
    assert_shows(&shown, "demi video generate: running\n");
    assert_shows(&shown, "demi video generate: the job job-1 is still running at Grok after ");
    assert_shows(&shown, "it runs to its end there\nexit=1");
    assert_eq!(video_rows(&harness).pop().unwrap().outcome.as_deref(), Some("failed"));

    // The bound counts from the ask, its own wait included: an ask that
    // took longer leaves no time to wait for the job.
    *videos.ask_takes.lock().unwrap() = Duration::from_millis(400);
    let shown = ran(&scripts, &mut socket, "m2", "demi video generate 'x' --model clip-1 > b.mp4; echo exit=$?").await;
    assert_shows(&shown, "demi video generate: the job job-2 is still running at Grok after ");
    let second = &shown[shown.find("job job-2").expect("the second job is named")..];
    assert!(!second.contains("demi video generate: running"), "{second}");
    backend.close().await;
}

// About a second: a login and a real device's shell jobs of two turns,
// the second holding the ledger for its job's whole bound of 300 ms.
//
// Planted defects: the cost written only once the video is downloaded, so
// a download that fails records nothing paid; the row written under the
// bound, so a bound that passes while it is written drops the row and
// panics on writing it again.
#[tokio::test]
async fn a_jobs_row_keeps_what_was_paid_and_is_written_once_whenever_the_bound_passes() {
    const BOUND: Duration = Duration::from_millis(300);
    let videos = Arc::new(VideoScript::new(&["clip-1"], Bytes::from_static(CLIP)));
    let logins = video_family(&videos);
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _device, _root, _tree) =
        tree_among(&scripts, reading_within(BOUND), logins.families.clone()).await;
    logins.login.signs_in("first");
    device_entry(&backend, &master, &logins).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The vendor reports the job done and its cost; the download fails.
    videos.answer([Ok(VideoScript::done())]);
    *videos.download_fails.lock().unwrap() =
        Some(MediaFailure::Unanswered { cause: "HTTP 502".into(), code: None, trace: Default::default() });
    let shown = ran(&scripts, &mut socket, "m1", "demi video generate 'x' --model clip-1 > a.mp4; echo exit=$?").await;
    assert_shows(&shown, "exit=1");
    let row = video_rows(&harness).pop().unwrap();
    assert_eq!((row.outcome.as_deref(), row.cost, row.cost_currency.as_deref()), (Some("failed"), Some(0.05), Some("usd")));

    // The job ends while the ledger is held, and the bound passes before
    // its row can be written.
    let held = async {
        let mut counts = videos.counts.subscribe();
        counts.wait_for(|&(asked, read)| asked == 2 && read >= 2).await.unwrap();
        let ledger = harness.control_database();
        ledger.execute_batch("BEGIN IMMEDIATE").unwrap();
        videos.answer([Ok(VideoScript::done())]);
        // The bound's passing is what is under test, and nothing announces
        // it: the ledger is held for a whole bound, which began at the ask.
        tokio::time::sleep(BOUND).await;
        ledger.execute_batch("COMMIT").unwrap();
    };
    let (shown, ()) = tokio::join!(
        ran(&scripts, &mut socket, "m2", "demi video generate 'x' --model clip-1 > b.mp4; echo exit=$?"),
        held
    );
    assert_shows(&shown, "1 s 64×36 MP4 with sound from clip-1 (");
    assert_shows(&shown, "exit=0");
    let row = video_rows(&harness).pop().unwrap();
    assert_eq!((row.outcome.as_deref(), row.cost, row.media_seconds.is_some()), (Some("answered"), Some(0.05), true));
    backend.close().await;
}

// About a second: one scripted server and a real device's shell jobs of
// three turns, the second woken by the user's decision.
//
// Planted defects: a video shape stored for a compatible entry without the
// permission; a video model added to an entry with no video shape; the
// 202 of OpenRouter's ask read as a failure; its video downloaded without
// the key; a stopped OpenAI Videos job not deleted at the server.
#[tokio::test]
async fn a_compatible_api_makes_videos_in_the_shape_the_agent_chose_and_cancels_where_it_can() {
    let vendor = MockVendor::start().await;
    let json_answer = |status: u16, body: serde_json::Value| {
        MockResponse::status(status).header("content-type", "application/json").chunk(body.to_string())
    };
    vendor.respond_at("/v1/videos", json_answer(202, json!({ "id": "job-1", "status": "pending" })));
    vendor.respond_at("/v1/videos/job-1", json_answer(200, json!({ "id": "job-1", "status": "in_progress" })));
    let content = vendor.url("/v1/videos/job-1/content?index=0");
    vendor.respond_at("/v1/videos/job-1", json_answer(200, json!({ "id": "job-1", "status": "completed",
        "unsigned_urls": [content], "usage": { "cost": 0.25 } })));
    vendor.respond_at("/v1/videos/job-1/content", MockResponse::status(200).chunk(Bytes::from_static(CLIP)));
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _device, root, _tree) =
        tree_among(&scripts, reading_within(Duration::from_secs(60)), demi_backend::families::builtin()).await;
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
    assert_eq!((proxy.media.video.shape, proxy.media.video.enabled), (None, true));
    std::fs::write(format!("{root}/expected.mp4"), CLIP).unwrap();
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The first change waits for the permission; once allowed it runs, and
    // a model of a kind the entry does not make is refused.
    scripts.root(
        FIRST,
        vec![
            background(
                "t1",
                "demi providers models add Proxy --id wan-2.2 --name 'Wan 2.2' --kind video; echo exit=$?",
            ),
            say("asked"),
            say("refused"),
            say("refused"),
        ],
    );
    socket.chat("m1", "Film").await;
    assert_shows(
        &last_root(&scripts),
        "waiting for the user's permission to manage providers; its end will report to you, so do not run it again",
    );
    let requested = permissions(&backend, &master, FIRST).await;
    let allowed = decide(&backend, &master, FIRST, requested.requests[0].id().as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&allowed.body));
    settled(
        &scripts,
        &backend,
        &master,
        FIRST,
        &["The user allowed this conversation to manage providers.", "(t1) ended with exit code 0"],
    )
    .await;
    // A socket of its own, which no frame of the woken turns precedes.
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root_anew(
        FIRST,
        vec![
            shell(
                "t2",
                &[
                    "demi providers edit Proxy --video openrouter-videos > /dev/null",
                    "demi providers models add Proxy --id wan-2.2 --name 'Wan 2.2' --kind video > /dev/null",
                    "demi providers show Proxy",
                    "demi video models",
                    "demi video generate 'a paper boat' --model wan-2.2 --duration 4 > boat.mp4 && cmp boat.mp4 expected.mp4 && echo same",
                ]
                .join("\n"),
            ),
            say("filmed"),
        ],
    );
    socket.chat("m2", "Film again").await;
    let shown = last_root(&scripts);
    assert_shows(&shown, "demi providers models add: models: wan-2.2 is a video model, but Proxy makes no videos; choose a video shape first\nexit=1");
    assert_shows(&shown, "Videos: openrouter-videos, Video Generation on\n");
    assert_shows(&shown, "Video models:\n  ID       NAME\n  wan-2.2  Wan 2.2\n");
    assert_shows(&shown, "ID       VENDOR  RELEASED  ENTRY  NAME\nwan-2.2  -       -         Proxy  Wan 2.2\n");
    assert_shows(
        &shown,
        "demi video generate: asked wan-2.2 (Proxy), job job-1\n\
         demi video generate: running\n\
         1 s 64×36 MP4 with sound from wan-2.2 (Proxy), $0.25\n\
         demi video generate: warning: asked for 4 s, got 1 s\n\
         same",
    );
    let sent = vendor.requests();
    assert_eq!((sent[0].uri.path(), sent[0].header("authorization")), ("/v1/videos", Some("Bearer sk-proxy")));
    assert_eq!(sent[0].json(), json!({ "model": "wan-2.2", "prompt": "a paper boat", "duration": 4 }));
    let download = sent.last().unwrap();
    assert_eq!((download.uri.path(), download.header("authorization")), ("/v1/videos/job-1/content", Some("Bearer sk-proxy")));
    let row = video_rows(&harness).pop().unwrap();
    assert_eq!(
        (row.provider_id.as_str(), row.account_id.as_deref(), row.outcome.as_deref(), row.cost, row.media_seconds),
        (proxy.id.as_str(), None, Some("answered"), Some(0.25), Some(1.0))
    );

    // OpenAI Videos cancels: a stopped call deletes its job at the server.
    vendor.respond_at("/v1/videos", json_answer(200, json!({ "id": "video_1", "status": "queued" })));
    for _ in 0..1000 {
        vendor.respond_at("/v1/videos/video_1", json_answer(200, json!({ "id": "video_1", "status": "queued" })));
    }
    let edited = backend
        .patch(&format!("/api/providers/{}", proxy.id), &master, json!({ "media": { "video": { "shape": "openai-videos" } } }))
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", String::from_utf8_lossy(&edited.body));
    let before = vendor.requests().len();
    scripts.root(
        FIRST,
        vec![
            shell(
                "t3",
                "demi video generate 'a boat' --model wan-2.2 > held.mp4 & call=$!; until [ -f go ]; do sleep 0.05; done; kill $call; wait $call; echo stopped",
            ),
            say("stopped"),
        ],
    );
    socket.send(&crate::conversations::send("m3", "Film and stop")).await;
    vendor.received(before + 2).await;
    std::fs::write(format!("{root}/go"), "").unwrap();
    socket.until_idle().await;
    crate::support::eventually("the stopped job is deleted at the server", async || {
        vendor.requests().iter().any(|request| request.method == "DELETE" && request.uri.path() == "/v1/videos/video_1")
    })
    .await;
    crate::support::eventually("the stopped call's row is written", async || {
        video_rows(&harness).last().and_then(|row| row.outcome.clone()).as_deref() == Some("cancelled")
    })
    .await;
    backend.close().await;
}
