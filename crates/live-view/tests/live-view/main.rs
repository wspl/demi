//! The live view's rate control on a simulated path (`live_view::testing`):
//! the crate's test binary. A page that falls behind in taking what arrives
//! looks to the delay estimate like a queue on the path; it must not cost the
//! picture its sharpness, while a path that really slows down must still be
//! followed at once.

use std::time::Duration;

use demi_live_view::testing::{Scenario, run};

const SECOND: Duration = Duration::from_secs(1);

#[test]
fn a_page_that_falls_behind_keeps_the_picture_sharp() {
    // Motion for 4 s, then a black screen's few hundred bytes a frame, as a
    // fold shows; from 6 s the page loses half a second each second for
    // 0.8 s, its main thread busy with the fold's layout, then catches up.
    let samples = run(&Scenario {
        picture: Box::new(|at| if at < 4 * SECOND { 40_000 } else { 850 }),
        capacity: Vec::new(),
        page_behind: Some((6 * SECOND, Duration::from_millis(800), 0.5)),
        until: 20 * SECOND,
    });
    for sample in samples.iter().filter(|sample| sample.at >= 6 * SECOND) {
        assert!(
            sample.fps == 60 && sample.scale == 1.0,
            "at {:?} the picture fell to {} fps at {} of its size, {} bps",
            sample.at,
            sample.fps,
            sample.scale,
            sample.target
        );
    }
}

#[test]
fn a_path_that_slows_down_is_followed() {
    // A moving picture of about 8 Mbps on a fast path that falls to 3 Mbps at
    // 6 s, as a relay's path does.
    let samples = run(&Scenario {
        picture: Box::new(|_| 16_000),
        capacity: vec![(Duration::ZERO, 1e9), (6 * SECOND, 3e6)],
        page_behind: None,
        until: 12 * SECOND,
    });
    let after = || samples.iter().filter(|sample| sample.at >= 6 * SECOND);
    let followed = after()
        .find(|sample| sample.target <= 3_500_000)
        .expect("the rate came down to the path's");
    assert!(followed.at <= 6 * SECOND + Duration::from_millis(1500), "the rate came down at {:?}", followed.at);
    let worst = after().map(|sample| sample.queue).max().unwrap_or_default();
    assert!(worst <= Duration::from_millis(1500), "pictures waited {worst:?} on the path");
}

