//! `demi browser record` against the pinned Chrome: a tab's frames become a
//! recording whose frames, images and video the commands read, and the
//! recording's limits and ends
//! (`crates/plugin-browser/command-chrome/src/live/record.rs`).

use std::time::Duration;

use serde_json::{Value, json};

use crate::families::BrowserFixture;

/// A change of one or two frames is in the listing, its image shows it under
/// a label, a still page costs no frames, and the video is an MP4.
///
/// Planted defects this catches: a recording that skips frames while the
/// page animates, a listing that keeps frames equal to the one before, an
/// export without its label, and a stop that loses the last frames.
///
/// Cost: one Chrome, about ten seconds.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; exercises a real browser"]
async fn a_tab_recording_keeps_the_frames_that_changed() {
    crate::families::with_recording_fixture(|fixture| async move {
        let tab = fixture.open("record.html").await;
        let started = fixture.call("browser.record", json!({ "target": tab, "action": "start" })).await;
        let id = started["recording"]["id"].as_str().unwrap().to_owned();
        // A still page, then a flash of two frames, then a slide of sixty.
        tokio::time::sleep(Duration::from_millis(500)).await;
        click(&fixture, &tab, "#flash").await;
        tokio::time::sleep(Duration::from_millis(700)).await;
        click(&fixture, &tab, "#nudge").await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        click(&fixture, &tab, "#slide").await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let stopped = fixture.call("browser.record", json!({ "target": id, "action": "stop" })).await;
        assert_eq!(stopped["recording"]["ended"], "stopped", "{stopped}");
        let frames = stopped["recording"]["frames"].as_u64().unwrap();
        assert!(frames >= 60, "the slide alone is sixty frames: {stopped}");
        assert!(frames < 200, "the still stretches cost no frames: {stopped}");

        let listed = fixture.call("browser.record", json!({ "target": id, "action": "frames" })).await;
        let rows = listed["frames"].as_array().unwrap();
        assert_eq!(rows.len() as u64, frames);
        // Whether the frame's changed area holds the rectangle.
        let covers = |row: &Value, x: i64, y: i64, width: i64, height: i64| {
            let area = &row["changed"];
            let (left, top) = (area["x"].as_i64().unwrap(), area["y"].as_i64().unwrap());
            let (across, down) = (area["width"].as_i64().unwrap(), area["height"].as_i64().unwrap());
            left <= x && top <= y && left + across >= x + width && top + down >= y + height
        };
        // The flash lasts two drawn frames: the first frame is the still page,
        // the next two are the square appearing and going, a frame apart, and
        // the still stretch after them costs none.
        let flash = &rows[1..3];
        assert!(flash.iter().all(|row| covers(row, 200, 100, 40, 40)), "the flash's two frames are listed: {listed}");
        let at = |row: &Value| row["timeUs"].as_u64().unwrap();
        assert!((25_000..45_000).contains(&(at(&rows[2]) - at(&rows[1]))), "the flash lasted two frames: {listed}");
        assert!(at(&rows[3]) - at(&rows[2]) > 400_000, "nothing is kept while the page is still: {listed}");
        // The one-pixel move of the dot shows as a frame of its own.
        assert!(
            rows.iter().any(|row| row["whole"] == false && covers(row, 300, 300, 7, 6)),
            "the nudge is listed: {listed}"
        );

        let number = flash[0]["frame"].as_u64().unwrap();
        let label = format!("f{number:03}");
        let exported = fixture
            .call("browser.record", json!({ "target": id, "action": "export", "frames": label }))
            .await;
        let path = exported["images"][0]["path"].as_str().unwrap();
        let image = image::open(path).unwrap().to_rgba8();
        // The label's strip above the area that changed.
        assert!(image.height() > 40 && image.width() >= 40, "{}x{}", image.width(), image.height());
        let red = (0..image.height()).any(|y| (0..image.width()).any(|x| {
            let pixel = image.get_pixel(x, y);
            pixel[0] > 180 && pixel[1] < 80 && pixel[2] < 80
        }));
        assert!(red, "the flash's square is red in {path}");

        // The video is an MP4, by command and by file.
        let saved = fixture.root.path().join("clip.mp4");
        let by_file = fixture
            .call(
                "browser.record",
                json!({ "target": id, "action": "export", "mp4": true, "output": saved.to_str().unwrap() }),
            )
            .await;
        assert_eq!(by_file["path"], saved.to_str().unwrap());
        let bytes = std::fs::read(&saved).unwrap();
        assert_eq!(&bytes[4..8], b"ftyp");
        fixture
    })
    .await;
}

async fn click(fixture: &BrowserFixture, tab: &str, css: &str) {
    fixture
        .call("browser.click", json!({ "tab": tab, "css": css, "timeout": 20000 }))
        .await;
}

/// A recording with a duration ends by itself and says so, one longer than
/// two minutes is refused, a tab can record once at a time, closing the tab
/// ends its recording, and releasing the conversation removes it.
///
/// Cost: one Chrome, about ten seconds.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; exercises a real browser"]
async fn a_tab_recording_ends_by_its_duration_and_with_its_tab() {
    crate::families::with_recording_fixture(|fixture| async move {
        let tab = fixture.open("record.html").await;
        let (code, refused) = fixture
            .result(
                "browser.record",
                json!({ "target": tab, "action": "start", "duration": "121s" }),
                tokio_util::sync::CancellationToken::new(),
            )
            .await;
        assert_eq!(code, 2, "{refused}");
        let started = fixture
            .call("browser.record", json!({ "target": tab, "action": "start", "duration": "1s" }))
            .await;
        let first = started["recording"]["id"].as_str().unwrap().to_owned();
        let (code, twice) = fixture
            .result(
                "browser.record",
                json!({ "target": tab, "action": "start" }),
                tokio_util::sync::CancellationToken::new(),
            )
            .await;
        assert_eq!(code, 1, "{twice}");
        assert_eq!(twice["error"]["code"], "recording");
        tokio::time::sleep(Duration::from_millis(2500)).await;
        let ended = fixture.call("browser.record", json!({ "target": first, "action": "stop" })).await;
        assert_eq!(ended["recording"]["ended"], "duration", "{ended}");
        assert!(ended["recording"]["frames"].as_u64().unwrap() >= 1);
        let ran = ended["recording"]["durationMs"].as_u64().unwrap();
        // It ran from its first frame, which the capture takes a moment to
        // deliver, to the end of the second it was given.
        assert!((700..1300).contains(&ran), "it ran {ran} ms");

        // The tab a recording runs on stops it, as the recording's ID does.
        let (code, idle) = fixture
            .result(
                "browser.record",
                json!({ "target": tab, "action": "stop" }),
                tokio_util::sync::CancellationToken::new(),
            )
            .await;
        assert_eq!(code, 1, "{idle}");
        assert_eq!(idle["error"]["code"], "recording_not_found");
        fixture.call("browser.record", json!({ "target": tab, "action": "start" })).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let by_tab = fixture.call("browser.record", json!({ "target": tab, "action": "stop" })).await;
        assert_eq!(by_tab["recording"]["ended"], "stopped", "{by_tab}");

        // Closing the tab ends the recording that runs on it.
        let started = fixture.call("browser.record", json!({ "target": tab, "action": "start" })).await;
        let second = started["recording"]["id"].as_str().unwrap().to_owned();
        tokio::time::sleep(Duration::from_millis(500)).await;
        fixture.call("browser.close", json!({ "tab": tab })).await;
        let closed = fixture.call("browser.record", json!({ "target": second, "action": "stop" })).await;
        assert_eq!(closed["recording"]["ended"], "closed", "{closed}");
        assert!(closed["recording"]["frames"].as_u64().unwrap() >= 1);
        // A recording outlasts its tab and its browser.
        fixture.call("browser.record", json!({ "target": second, "action": "frames" })).await;
        let gone = fixture.result(
            "browser.record",
            json!({ "target": "r99", "action": "frames" }),
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
        assert_eq!(gone.1["error"]["code"], "recording_not_found");
        // One that never was is not said to have been removed.
        assert!(!gone.1["error"]["message"].as_str().unwrap().contains("removed"), "{}", gone.1);
        fixture
    })
    .await;
}

/// A recorded tab that draws nothing can be resized: the resize waits for the
/// capture to be released, and the recording gives it up at once instead of
/// when another frame comes, then goes on at the new size.
///
/// Planted defect this catches: a recording that waits for a resize only if
/// one has begun when it began waiting, so that a resize of a still tab waits
/// until the recording ends.
///
/// Cost: one Chrome, about five seconds.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; exercises a real browser"]
async fn a_still_tab_that_is_recorded_can_be_resized() {
    crate::families::with_recording_fixture(|fixture| async move {
        let tab = fixture.open("record.html").await;
        let started = fixture.call("browser.record", json!({ "target": tab, "action": "start" })).await;
        let id = started["recording"]["id"].as_str().unwrap().to_owned();
        // The page has drawn its first frame and goes still.
        tokio::time::sleep(Duration::from_secs(2)).await;
        tokio::time::timeout(
            Duration::from_secs(15),
            fixture.call("browser.viewport.set", json!({ "tab": tab, "width": 600, "height": 400 })),
        )
        .await
        .expect("the resize does not wait for the recording to end");
        click(&fixture, &tab, "#flash").await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let stopped = fixture.call("browser.record", json!({ "target": id, "action": "stop" })).await;
        assert_eq!(stopped["recording"]["ended"], "stopped", "{stopped}");
        let listed = fixture.call("browser.record", json!({ "target": id, "action": "frames" })).await;
        let rows = listed["frames"].as_array().unwrap();
        // The tab's new size starts the video's next stretch, whole.
        assert!(rows.iter().skip(1).any(|row| row["whole"] == true), "the resize is a frame of its own: {listed}");
        assert_eq!(stopped["recording"]["size"]["width"], listed["recording"]["size"]["width"]);
        fixture
    })
    .await;
}

/// Closing the last tab ends its recording complete before the close is
/// answered: the frames drawn up to the close, and the end, are in the files
/// when the service may end.
///
/// Planted defect this catches: a browser that stops before the recording
/// task finishes, which leaves the recording without its end, to be read as
/// interrupted.
///
/// Cost: one Chrome, about five seconds.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; exercises a real browser"]
async fn closing_the_last_tab_completes_its_recording_first() {
    crate::families::with_recording_fixture(|fixture| async move {
        let tab = fixture.open("record.html").await;
        let started = fixture.call("browser.record", json!({ "target": tab, "action": "start" })).await;
        let id = started["recording"]["id"].as_str().unwrap().to_owned();
        tokio::time::sleep(Duration::from_millis(500)).await;
        click(&fixture, &tab, "#slide").await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        fixture.call("browser.close", json!({ "tab": tab })).await;
        // From the files, which nothing is left to complete: no stop to wait for.
        let directory = demi_command_protocol::recordings_directory(
            Some(&fixture.root.path().join("home")),
            &fixture.conversation,
        )
        .join(&id);
        let meta: Value = serde_json::from_slice(&std::fs::read(directory.join("recording.json")).unwrap()).unwrap();
        assert_eq!(meta["ended"]["reason"], "closed", "{meta}");
        let index = std::fs::read_to_string(directory.join("frames.jsonl")).unwrap();
        assert!(index.lines().all(|line| line.contains("\"k\":\"f\"") || line.contains("\"k\":\"m\"")), "the pictures are decoded into frames");
        let listed = fixture.call("browser.record", json!({ "target": id, "action": "frames" })).await;
        assert!(listed["recording"]["frames"].as_u64().unwrap() > 10, "{listed}");
        fixture
    })
    .await;
}

/// A stop right after a change keeps the change: the pictures the encoder
/// holds when the stop comes are flushed and delivered.
///
/// Planted defect this catches: a stop that retires the recording before its
/// encoder's flush, which drops the last pictures.
///
/// Cost: one Chrome, about five seconds.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; exercises a real browser"]
async fn a_stop_right_after_a_change_keeps_the_change() {
    crate::families::with_recording_fixture(|fixture| async move {
        let tab = fixture.open("record.html").await;
        let started = fixture.call("browser.record", json!({ "target": tab, "action": "start" })).await;
        let id = started["recording"]["id"].as_str().unwrap().to_owned();
        tokio::time::sleep(Duration::from_secs(1)).await;
        click(&fixture, &tab, "#slide").await;
        // The slide's last pixel is drawn about a second later; stop as it is.
        tokio::time::sleep(Duration::from_millis(1000)).await;
        let stopped = fixture.call("browser.record", json!({ "target": id, "action": "stop" })).await;
        let listed = fixture.call("browser.record", json!({ "target": id, "action": "frames" })).await;
        let rows = listed["frames"].as_array().unwrap();
        let last = rows.last().unwrap();
        // The runner reaches x = 60, so its last frame's area ends at 90.
        let area = &last["changed"];
        let right = area["x"].as_i64().unwrap() + area["width"].as_i64().unwrap();
        assert!(right >= 89, "the last frames of the slide are kept: {stopped} {last}");
        fixture
    })
    .await;
}
