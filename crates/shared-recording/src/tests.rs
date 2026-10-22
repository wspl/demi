//! A recording run through its own interface with a real encoder and a source
//! that does what every source does: drop a frame equal to the one before,
//! code the others, say where they differ.

use std::sync::Arc;

use demi_shared_h264::{PixelFormat, Raw, Rate, Settings};
use demi_shared_openh264::{Built, Codec as _};
use demi_shared_recording_protocol::{Area, EndReason, FrameSet, KEPT};

use crate::{Crop, Error, Export, Library, Picture, Sink};

const WIDTH: u32 = 96;
const HEIGHT: u32 = 64;

/// A grey screen with a red square at `at`, if any, as RGBA.
fn screen(at: Option<(u32, u32)>) -> Vec<u8> {
    let mut pixels = vec![90u8; (WIDTH * HEIGHT * 4) as usize];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    if let Some((x, y)) = at {
        for row in y..y + 8 {
            for column in x..x + 8 {
                let at = ((row * WIDTH + column) * 4) as usize;
                pixels[at..at + 4].copy_from_slice(&[230, 20, 20, 255]);
            }
        }
    }
    pixels
}

/// A source of a recording: OpenH264 in software, so that the tests code the
/// same on every machine.
struct Source {
    sink: Sink,
    encoder: Box<dyn demi_shared_h264::Encoder>,
    before: Option<Vec<u8>>,
}

impl Source {
    async fn new(sink: Sink) -> Self {
        let load = Built.library().await.unwrap();
        let settings = Settings {
            width: WIDTH,
            height: HEIGHT,
            rate: Rate::Quality,
        };
        Self {
            sink,
            encoder: Box::new(demi_shared_h264::software(settings, &load).unwrap()),
            before: None,
        }
    }

    /// The screen drew `pixels` at `millis`. Whether it differed from the
    /// frame before, so was kept.
    fn draw(&mut self, pixels: Vec<u8>, millis: u64) -> bool {
        let now = raw(&pixels);
        let changed = match &self.before {
            Some(before) => crate::changed(&now, &raw(before)),
            None => Some(Area::whole(WIDTH, HEIGHT)),
        };
        let Some(changed) = changed else {
            return false;
        };
        let first = self.before.is_none();
        for coded in self.encoder.encode(now, millis * 1000, first).unwrap() {
            assert!(self.sink.push_picture(Picture {
                data: coded.data,
                key: coded.key,
                at_us: coded.time_us as f64,
                width: WIDTH,
                height: HEIGHT,
                changed,
            }));
        }
        self.before = Some(pixels);
        true
    }
}

fn raw(pixels: &[u8]) -> Raw<'_> {
    Raw {
        width: WIDTH,
        height: HEIGHT,
        format: PixelFormat::Rgba,
        pixels,
    }
}

fn library(directory: &std::path::Path) -> Library {
    Library::new(directory.to_owned(), Arc::new(Built))
}

/// A one-frame change is kept as surely as a long one and is found in the
/// listing, a frame equal to the one before costs nothing, and the frame
/// exported shows the change under a label of its number.
/// Planted defects this catches: a diff that compares only every other
/// frame, and an index that drops the last frame before the stop.
#[tokio::test]
async fn a_frame_that_shows_for_one_frame_is_kept_and_exported() {
    let directory = tempfile::tempdir().unwrap();
    let library = library(directory.path());
    let live = library.begin("t1", None).await.unwrap();
    let mut source = Source::new(live.sink()).await;
    // Still, a flash for one frame, still again, a square that moves.
    let frames = [
        screen(None),
        screen(None),
        screen(Some((40, 20))),
        screen(None),
        screen(None),
        screen(Some((10, 10))),
        screen(Some((14, 10))),
    ];
    let kept: Vec<bool> = frames.into_iter().enumerate().map(|(index, frame)| source.draw(frame, index as u64 * 16)).collect();
    assert_eq!(kept, [true, false, true, true, false, true, true]);
    let info = live.stop(EndReason::Stopped).await.unwrap();
    assert_eq!(info.ended, Some(EndReason::Stopped));
    assert_eq!((info.frames, info.missed), (5, 0), "the two repeats were not kept");
    let stored = library.stored(live.id()).await.unwrap();
    let load = Built.library().await.unwrap();
    let listing = stored.frames(None).unwrap();
    let flash = &listing.frames[1];
    assert_eq!(flash.time_us, 32_000);
    assert_eq!(flash.changed, Area { x: 40, y: 20, width: 8, height: 8 });
    let out = directory.path().join("images");
    let exported = stored
        .export(&Export {
            frames: Some(&"f002".to_owned().try_into().unwrap()),
            crop: Crop::Changed,
            directory: &out,
            overwrite: false,
            load: &load,
        })
        .unwrap();
    assert_eq!(exported.images.len(), 1);
    let picture = image::open(&exported.images[0].1).unwrap().to_rgba8();
    // The strip of the label is above the 8 × 8 area, which is all red.
    let (width, height) = picture.dimensions();
    let red = picture.get_pixel(2, height - 3);
    assert!(red[0] > 180 && red[1] < 90 && red[2] < 90, "the flash shows red: {red:?}");
    let strip_has_text = (0..width.min(80)).any(|x| (0..20).any(|y| picture.get_pixel(x, y) != picture.get_pixel(0, 0)));
    assert!(strip_has_text, "the label is drawn above the picture");
}

/// The frames of one export share one area, and a region or the whole frame
/// can be asked for; an area outside the frame is refused.
#[tokio::test]
async fn the_frames_of_one_export_share_an_area() {
    let directory = tempfile::tempdir().unwrap();
    let library = library(directory.path());
    let live = library.begin("t1", None).await.unwrap();
    let mut source = Source::new(live.sink()).await;
    for (index, at) in [None, Some((10, 10)), Some((60, 30))].into_iter().enumerate() {
        source.draw(screen(at), index as u64 * 20);
    }
    live.stop(EndReason::Stopped).await.unwrap();
    let stored = library.stored(live.id()).await.unwrap();
    let load = Built.library().await.unwrap();
    let out = directory.path().join("images");
    let size = |crop| {
        let exported = stored
            .export(&Export {
                frames: Some(&FrameSet::try_from("f002-f003".to_owned()).unwrap()),
                crop,
                directory: &out,
                overwrite: true,
                load: &load,
            })
            .unwrap();
        let sizes: Vec<_> = exported.images.iter().map(|(_, path)| image::image_dimensions(path).unwrap()).collect();
        assert_eq!(sizes[0], sizes[1]);
        (exported.area, sizes[0])
    };
    assert_eq!(size(Crop::Changed).0, Area { x: 10, y: 10, width: 58, height: 28 });
    assert_eq!(size(Crop::Whole).0, Area::whole(WIDTH, HEIGHT));
    let region = Area { x: 0, y: 0, width: 20, height: 20 };
    assert_eq!(size(Crop::Region(region)).0, region);
    let outside = stored.export(&Export {
        frames: None,
        crop: Crop::Region(Area { x: 500, y: 0, width: 5, height: 5 }),
        directory: &out,
        overwrite: true,
        load: &load,
    });
    assert!(matches!(outside, Err(Error::BadRequest(_))), "{:?}", outside.err());
}

/// A frame the capture could not take is counted, and where it fell is in
/// the listing.
#[tokio::test]
async fn missed_frames_are_counted_where_they_fell() {
    let directory = tempfile::tempdir().unwrap();
    let library = library(directory.path());
    let live = library.begin("t1", None).await.unwrap();
    let mut source = Source::new(live.sink()).await;
    source.draw(screen(None), 0);
    live.sink().miss(3);
    source.draw(screen(Some((5, 5))), 50);
    let info = live.stop(EndReason::Closed).await.unwrap();
    assert_eq!((info.frames, info.missed, info.ended), (2, 3, Some(EndReason::Closed)));
    let listing = library.stored(live.id()).await.unwrap().frames(None).unwrap();
    assert_eq!(listing.gaps.len(), 1);
    assert_eq!((listing.gaps[0].after, listing.gaps[0].count), (1, 3));
}

/// A recording another program writes is not removed to keep the newest five:
/// two programs, a browser's and a phone's, record for one conversation in one
/// directory, and each prunes it.
/// Planted defect this catches: a prune that removes the oldest recording not
/// held by its own program, which another program is still writing.
#[tokio::test]
async fn a_recording_another_program_writes_is_not_pruned() {
    let directory = tempfile::tempdir().unwrap();
    let one = library(directory.path());
    let other = library(directory.path());
    let running = one.begin("t1", None).await.unwrap();
    Source::new(running.sink()).await.draw(screen(None), 0);
    for _ in 0..KEPT {
        let live = other.begin("m1", None).await.unwrap();
        Source::new(live.sink()).await.draw(screen(None), 0);
        live.stop(EndReason::Stopped).await.unwrap();
    }
    other.stored(running.id()).await.expect("the recording the other program writes stays");
    assert!(directory.path().join(running.id().as_str()).is_dir());
    running.stop(EndReason::Stopped).await.unwrap();
    // Once it ended, it is the oldest and goes with the next.
    let live = other.begin("m1", None).await.unwrap();
    live.stop(EndReason::Stopped).await.unwrap();
    assert!(matches!(other.stored(running.id()).await, Err(Error::Removed(_))));
}

/// The video is an MP4 that plays at the pace it was recorded: the last
/// frame holds until the end.
#[tokio::test]
async fn the_video_is_an_mp4_that_plays_as_long_as_it_ran() {
    let directory = tempfile::tempdir().unwrap();
    let library = library(directory.path());
    let live = library.begin("t1", None).await.unwrap();
    let mut source = Source::new(live.sink()).await;
    for (index, at) in [None, Some((10, 10)), Some((20, 10))].into_iter().enumerate() {
        source.draw(screen(at), index as u64 * 100);
    }
    live.stop(EndReason::Stopped).await.unwrap();
    let stored = library.stored(live.id()).await.unwrap();
    let path = directory.path().join("video.mp4");
    let ran = stored.write_mp4(&path).unwrap();
    let facts = demi_shared_types::mp4::read(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((facts.width, facts.height), (WIDTH, HEIGHT));
    assert!((facts.seconds - ran.as_secs_f64()).abs() < 0.01, "{} against {ran:?}", facts.seconds);
    assert!(facts.seconds >= 0.2);
}

mod listing {
    use demi_shared_recording_protocol::{
        Area, FrameRow, Gap, RecordingId, RecordingInfo,
        text::listing,
    };

    fn info(frames: u64) -> RecordingInfo {
        RecordingInfo {
            id: "r1".parse::<RecordingId>().expect("an id"),
            target: "t1".into(),
            running: false,
            ended: None,
            note: None,
            duration_ms: 1000,
            frames,
            missed: 0,
            area: None,
            size: None,
        }
    }

    fn row(frame: u32) -> FrameRow {
        FrameRow {
            frame,
            time_us: u64::from(frame) * 16_000,
            changed: Area { x: 1, y: 2, width: 3, height: 4 },
            whole: false,
        }
    }

    /// The listing's frame and gap rows, in order.
    fn rows(text: &str) -> Vec<&str> {
        text.lines().filter(|line| line.starts_with('f') || line.contains("missed")).collect()
    }

    /// Planted defect this catches: a gap just before the first listed frame
    /// that stands at the head of the gaps and hides every later one.
    #[test]
    fn a_gap_before_the_first_listed_frame_hides_none_after_it() {
        let gaps = [Gap { after: 4, count: 2 }, Gap { after: 6, count: 3 }, Gap { after: 8, count: 1 }];
        let text = listing(&info(9), &[row(5), row(6), row(7), row(8)], &gaps, "demi browser record");
        let rows = rows(&text);
        // The gap after f004 was listed with the frames before.
        assert_eq!(rows.len(), 6, "{text}");
        assert!(rows[1].starts_with("f006") && rows[2].contains("(3 missed)"), "{rows:?}");
        assert!(rows[4].starts_with("f008") && rows[5].contains("(1 missed)"), "{rows:?}");
    }

    /// A gap before the first frame is listed before it, and one after a frame
    /// that is not listed, before the next listed one.
    #[test]
    fn a_gap_before_the_first_frame_and_one_after_an_unlisted_frame_are_listed() {
        let gaps = [Gap { after: 0, count: 5 }, Gap { after: 2, count: 7 }];
        let text = listing(&info(4), &[row(1), row(3)], &gaps, "demi browser record");
        let rows = rows(&text);
        assert!(rows[0].contains("(5 missed)") && rows[1].starts_with("f001"), "{rows:?}");
        assert!(rows[2].contains("(7 missed)") && rows[3].starts_with("f003"), "{rows:?}");
    }
}

mod held {
    use demi_shared_recording_protocol::Area;

    use crate::{Held, Settled};

    fn area(x: u32, y: u32, width: u32, height: u32) -> Area {
        Area { x, y, width, height }
    }

    /// The picture the encoder gives after one it dropped differs from the one
    /// before the dropped, in its own area and the dropped one's, and the
    /// dropped one is counted.
    /// Planted defect this catches: a dropped picture whose area is lost, so
    /// the next picture says it differs only where it does from the dropped.
    #[test]
    fn a_picture_after_a_dropped_one_differs_in_both_areas() {
        let mut held = Held::default();
        held.give(10, area(0, 0, 10, 10));
        held.give(20, area(50, 50, 10, 10));
        held.give(30, area(5, 5, 4, 4));
        let settled = held.settle(&[10, 30], &[20]);
        assert_eq!(
            settled,
            [
                Settled::Given { at: 10, changed: area(0, 0, 10, 10) },
                Settled::Dropped,
                Settled::Given { at: 30, changed: area(5, 5, 55, 55) },
            ]
        );
    }

    /// A drop the encoder reports after the next picture was given is still
    /// carried to that picture, and a picture not answered yet stays held with
    /// every later one.
    #[test]
    fn an_unanswered_picture_stays_held_and_a_late_drop_is_carried() {
        let mut held = Held::default();
        held.give(10, area(0, 0, 2, 2));
        held.give(20, area(8, 8, 2, 2));
        held.give(30, area(4, 4, 2, 2));
        assert!(held.settle(&[], &[]).is_empty());
        // The encoder says the first was dropped; the second is not answered.
        assert_eq!(held.settle(&[], &[10]), [Settled::Dropped]);
        let settled = held.settle(&[20, 30], &[]);
        assert_eq!(
            settled,
            [
                Settled::Given { at: 20, changed: area(0, 0, 10, 10) },
                Settled::Given { at: 30, changed: area(4, 4, 2, 2) },
            ]
        );
    }
}
