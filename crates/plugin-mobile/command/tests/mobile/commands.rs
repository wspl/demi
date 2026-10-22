//! The timing of a command's reads of the scripted iPhone's screen.

use std::time::Duration;

use serde_json::json;

use crate::{CHAT, Mobile};

/// A screen that keeps changing is taken as it is after 1.5 s, and the
/// command says so.
#[tokio::test(start_paused = true)]
async fn a_screen_still_changing_is_taken_after_the_settle_limit() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let fake = mobile.driver.device("UDID-1");
    let drawing = tokio::spawn(async move {
        loop {
            fake.draw();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let shot = mobile
        .run(CHAT, "screenshot", json!({ "device": device, "output": "busy.png" }))
        .await;
    drawing.abort();
    assert_eq!(shot.json()["settled"], false);
    assert!(
        shot.stderr.contains("demi mobile screenshot: the screen was still changing after 1.5 s"),
        "{}",
        shot.stderr
    );
}

/// A wait ends at its timeout even while a read of the tree is under way:
/// a screen whose read takes 10 s fails a 2 s wait at 2 s. Paused time.
#[tokio::test(start_paused = true)]
async fn a_wait_ends_at_its_timeout_during_a_slow_read() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    mobile.driver.device("UDID-1").reads_take(Duration::from_secs(10));
    let started = tokio::time::Instant::now();
    let waited = mobile.run(CHAT, "wait", json!({ "device": device, "text": "Never", "timeout": 2 })).await;
    assert_eq!(waited.failure().0, "timeout");
    assert!(started.elapsed() < Duration::from_secs(3), "the wait took {:?}", started.elapsed());
}

