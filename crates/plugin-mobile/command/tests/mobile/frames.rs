//! The pieces of an Android emulator's frame read from files
//! (`crates/plugin-mobile/command/src/live.rs`): Android Studio's installations and the
//! device art in them, and the rounded-corner drawable SystemUI paints the
//! screen's corners with, compiled as its APK holds it.

use std::path::{Path, PathBuf};

use std::io::Write as _;

use demi_mobile::android::{
    art::{self, Art, Places, System},
    axml, corners,
    guest::{self, Resolved},
};

/// An installation's folder of art for `device`, with a layout in it.
fn install(installation: &Path, device: &str) -> PathBuf {
    let folder = installation.join("plugins/android/resources/device-art-resources").join(device);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("layout"), "parts {\n}\n").unwrap();
    folder
}

/// Android Studio is found where each system's installer, the user, or
/// JetBrains Toolbox puts it, a release before a preview, and the device's art
/// in the first installation that has it. A device name that is a path finds
/// nothing.
///
/// Planted defects this catches:
///
/// - a Mac's app looked for without its `Contents`;
/// - the user's own Applications folder left out;
/// - a name that climbs out of the art's folder.
///
/// Cost: temporary folders; milliseconds.
#[test]
fn android_studios_art_is_found_where_each_system_installs_it() {
    let root = tempfile::tempdir().unwrap();
    let places = Places {
        root: root.path().to_owned(),
        home: Some(root.path().join("home")),
        program_files: Some(root.path().join("Program Files")),
        local_app_data: Some(root.path().join("Local")),
    };

    let preview = install(&root.path().join("Applications/Android Studio Preview.app/Contents"), "pixel_10");
    let user = install(&root.path().join("home/Applications/Android Studio.app/Contents"), "pixel_9");
    let installations = art::installations(System::MacOs, &places);
    assert_eq!(art::find(&installations, "pixel_10"), Some(preview.clone()));
    assert_eq!(art::find(&installations, "pixel_9"), Some(user));
    let release = install(&root.path().join("Applications/Android Studio.app/Contents"), "pixel_10");
    assert_eq!(art::find(&art::installations(System::MacOs, &places), "pixel_10"), Some(release));
    assert_eq!(art::find(&installations, "medium_phone"), None);
    assert_eq!(art::find(&installations, "../pixel_10"), None);

    let windows = install(&root.path().join("Program Files/Android/Android Studio"), "pixel_8");
    let toolbox = install(&root.path().join("Local/Programs/Android Studio"), "pixel_7");
    let installations = art::installations(System::Windows, &places);
    assert_eq!(art::find(&installations, "pixel_8"), Some(windows));
    assert_eq!(art::find(&installations, "pixel_7"), Some(toolbox));

    let opt = install(&root.path().join("opt/android-studio"), "pixel_6");
    let home = install(&root.path().join("home/android-studio-2026.2"), "pixel_5");
    let jetbrains = install(&root.path().join("home/.local/share/JetBrains/Toolbox/apps/android-studio"), "pixel_4");
    let installations = art::installations(System::Linux, &places);
    assert_eq!(art::find(&installations, "pixel_6"), Some(opt));
    assert_eq!(art::find(&installations, "pixel_5"), Some(home));
    assert_eq!(art::find(&installations, "pixel_4"), Some(jetbrains));
}

/// Writes `product-info.json` for an installation of `channel` (such as
/// `AndroidStudio` or `AndroidStudioPreview`) at `build`, where a Mac's app
/// keeps it.
fn release(contents: &Path, channel: &str, build: &str) {
    let resources = contents.join("Resources");
    std::fs::create_dir_all(&resources).unwrap();
    let info = serde_json::json!({ "name": "Android Studio", "buildNumber": build, "dataDirectoryName": format!("{channel}2026.2") });
    std::fs::write(resources.join("product-info.json"), info.to_string()).unwrap();
}

/// Of several installations, a stable release is taken before a preview
/// wherever each is installed, and of two of the same channel the newer
/// build. Planted defects this catches: the first place's installation
/// taken whatever its release, or builds compared as text (262 before
/// 1000). Temporary folders; milliseconds.
#[test]
fn a_stable_release_is_taken_before_a_preview_then_the_newer_one() {
    let root = tempfile::tempdir().unwrap();
    let places = Places {
        root: root.path().to_owned(),
        home: Some(root.path().join("home")),
        ..Places::default()
    };
    let preview = root.path().join("Applications/Android Studio Preview.app/Contents");
    install(&preview, "pixel_10");
    release(&preview, "AndroidStudioPreview", "263.100.1");
    let older = root.path().join("home/Applications/Android Studio.app/Contents");
    let older_art = install(&older, "pixel_10");
    release(&older, "AndroidStudio", "262.9437.185");
    assert_eq!(art::find(&art::installations(System::MacOs, &places), "pixel_10"), Some(older_art));
    let newer = root.path().join("home/Applications/Android Studio 2.app/Contents");
    let newer_art = install(&newer, "pixel_10");
    release(&newer, "AndroidStudio", "1000.1");
    assert_eq!(art::find(&art::installations(System::MacOs, &places), "pixel_10"), Some(newer_art));
}

/// A layout is read as Android Studio reads it:
///
/// - comments are skipped;
/// - the screen's size comes from the device part;
/// - the first layout that shows the device upright is used (a landscape one
///   that turns the device is passed over);
/// - the frame's image is placed by the parts' positions and cut away over the
///   screen, and a frame clear at the screen's centre is its own mask;
/// - the buttons are drawn where the part places them;
/// - the art is cropped to what it shows.
///
/// Planted defects this catches:
///
/// - the rotated layout taken;
/// - the frame's image left over the screen;
/// - the buttons' places taken from the art's corner.
///
/// Cost: images written by the test; milliseconds.
#[test]
fn a_device_arts_layout_places_the_screen_in_its_frame() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("layout"),
        "# An old phone, with a button under its screen.\n\
         parts {\n  device {\n    display {\n      width 40\n      height 60\n    }\n  }\n\
           portrait {\n    background {\n      image back.png\n    }\n\
             buttons {\n      home {\n        image button.png\n        x 25\n        y 85\n      }\n    }\n  }\n}\n\
         layouts {\n\
           landscape {\n    width 120\n    height 70\n    part1 {\n      name portrait\n    }\n\
             part2 {\n      name device\n      x 30\n      y 50\n      rotation 3\n    }\n  }\n\
           portrait {\n    width 70\n    height 100\n    part1 {\n      name portrait\n      x 0\n      y 0\n    }\n\
             part2 {\n      name device\n      x 15\n      y 10\n    }\n  }\n}\n",
    )
    .unwrap();
    // A frame whose screen is cut out, clear for 2 pixels outside it all
    // round, and whose own screen corner is black.
    let mut back = image::RgbaImage::from_pixel(70, 100, image::Rgba([90, 90, 90, 255]));
    for y in 8..72 {
        for x in 13..57 {
            back.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
        }
    }
    back.put_pixel(15, 10, image::Rgba([0, 0, 0, 255]));
    back.save(folder.path().join("back.png")).unwrap();
    image::RgbaImage::from_pixel(20, 6, image::Rgba([200, 0, 0, 255])).save(folder.path().join("button.png")).unwrap();

    let art = Art::load(folder.path()).unwrap();
    assert_eq!(art.screen, (15, 10, 40, 60));
    assert_eq!(art.image.dimensions(), (70, 100));
    assert_eq!(art.image.get_pixel(15, 10).0, [0, 0, 0, 255], "the frame's own screen is its mask");
    assert_eq!(art.image.get_pixel(35, 40)[3], 0, "the picture shows through the screen");
    assert_eq!(art.image.get_pixel(14, 40)[3], 0, "the frame's clear margin stays clear");
    assert_eq!(art.image.get_pixel(5, 40).0, [90, 90, 90, 255]);
    assert_eq!(art.image.get_pixel(30, 88).0, [200, 0, 0, 255], "the button lies where the part places it");
}

/// The screen's edge across a corner, read from SystemUI's compiled drawable.
///
/// A Pixel 10's spans 196 pixels and passes through its curve's own points. A
/// Pixel 10 Pro's, drawn the other way round in a viewport of 34.3 units, spans
/// its 233 pixels.
///
/// Planted defects this catches:
///
/// - the viewport's units taken for pixels;
/// - the fill read inside out;
/// - the points out of order.
///
/// Cost: the drawables of the emulator's own overlays; a millisecond.
#[test]
fn a_corners_shape_is_read_from_systemuis_drawable() {
    let near = |points: &[(f64, f64)], (x, y): (f64, f64), within: f64| {
        points.windows(2).any(|pair| {
            let ((x1, y1), (x2, y2)) = (pair[0], pair[1]);
            let along = (((x - x1) * (x2 - x1) + (y - y1) * (y2 - y1)) / ((x2 - x1).powi(2) + (y2 - y1).powi(2))).clamp(0.0, 1.0);
            (x1 + along * (x2 - x1) - x).hypot(y1 + along * (y2 - y1) - y) < within
        })
    };

    let pixel_10 = corners::shape(include_bytes!("fixtures/rounded_corner.pixel_10.xml"), 2.625).unwrap();
    assert!(pixel_10.first().is_some_and(|&(x, y)| x < 0.5 && y > 195.0), "{pixel_10:?}");
    assert!(pixel_10.last().is_some_and(|&(x, y)| x > 195.0 && y < 0.5), "{pixel_10:?}");
    for on_curve in [(10.98, 91.35), (89.96, 11.51)] {
        assert!(near(&pixel_10, on_curve, 0.5), "{on_curve:?} is off {pixel_10:?}");
    }

    let pixel_10_pro = corners::shape(include_bytes!("fixtures/rounded_corner.pixel_10_pro.xml"), 2.8125).unwrap();
    let scale = 233.0 / 34.3;
    assert!(pixel_10_pro.first().is_some_and(|&(x, y)| x < 0.5 && y > 232.0), "{pixel_10_pro:?}");
    for on_curve in [(7.5, 7.5), (20.1, 0.8), (0.8, 20.1)] {
        assert!(near(&pixel_10_pro, (on_curve.0 * scale, on_curve.1 * scale), 0.5), "{on_curve:?} is off {pixel_10_pro:?}");
    }
}

/// The bytes of a compiled XML document made of `chunks`.
fn document(chunks: &[Vec<u8>]) -> Vec<u8> {
    let length = 8 + chunks.iter().map(Vec::len).sum::<usize>();
    let mut bytes = vec![0x03, 0x00, 0x08, 0x00];
    bytes.extend((length as u32).to_le_bytes());
    for chunk in chunks {
        bytes.extend(chunk);
    }
    bytes
}

/// A UTF-8 string pool of `count` strings, every one of them `text`.
fn pool_of_one(text: &str, count: u32) -> Vec<u8> {
    let header = 28u32;
    let start = header + count * 4;
    let mut data = vec![text.len() as u8 & 0x7f, text.len() as u8 & 0x7f];
    if text.len() >= 0x80 {
        // Two bytes for each length.
        let length = [0x80 | (text.len() >> 8) as u8, text.len() as u8];
        data = [length, length].concat();
    }
    data.extend(text.as_bytes());
    data.push(0);
    while data.len() % 4 != 0 {
        data.push(0);
    }
    let size = start + data.len() as u32;
    let mut chunk = vec![0x01, 0x00, header as u8, 0x00];
    for field in [size, count, 0, 1 << 8, start, 0] {
        chunk.extend(field.to_le_bytes());
    }
    for _ in 0..count {
        chunk.extend(0u32.to_le_bytes());
    }
    chunk.extend(data);
    chunk
}

/// The byte where the first element of the compiled XML `bytes` starts.
fn first_element(bytes: &[u8]) -> usize {
    let field = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    let mut at = 8;
    while u16::from_le_bytes([bytes[at], bytes[at + 1]]) != 0x0102 {
        at += field(at + 4);
    }
    at
}

/// The device's compiled drawable is read only as far as its bytes hold.
///
/// A stated length other than the bytes', attributes laid on one another or
/// past their element, and strings that name the same bytes over and over all
/// fail the read instead of growing it.
///
/// Planted defects this catches:
///
/// - the stated length cut to the bytes;
/// - 65,535 attributes read from one;
/// - a string copied once for each name of it.
///
/// Cost: built bytes; milliseconds.
#[test]
fn a_compiled_drawable_is_read_no_larger_than_its_bytes() {
    let fixture = include_bytes!("fixtures/rounded_corner.pixel_10.xml").to_vec();
    assert!(axml::events(&fixture).is_ok());

    let mut longer = fixture.clone();
    longer[4..8].copy_from_slice(&(fixture.len() as u32 + 1000).to_le_bytes());
    assert!(axml::events(&longer).is_err(), "a false length was taken");

    let element = first_element(&fixture);
    let header = u16::from_le_bytes([fixture[element + 2], fixture[element + 3]]) as usize;
    let mut stacked = fixture.clone();
    stacked[element + header + 10..element + header + 12].copy_from_slice(&0u16.to_le_bytes());
    stacked[element + header + 12..element + header + 14].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(axml::events(&stacked).is_err(), "attributes laid on one another were read");
    let mut past = fixture.clone();
    past[element + header + 12..element + header + 14].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(axml::events(&past).is_err(), "attributes past their element were read");

    let shared = document(&[pool_of_one(&"x".repeat(4000), 2000)]);
    assert!(axml::events(&shared).is_err(), "one string was read 2,000 times over");
    assert!(axml::events(&document(&[pool_of_one("vector", 3)])).is_ok());
}

/// A file is read from a copied APK where its directory places it and no
/// larger than its limit, and the copy is gone once read. Planted defects
/// this catches: a file read whole whatever its size, or the copy left on
/// the Host. A small archive of the test's; milliseconds.
#[test]
fn an_apks_file_is_read_no_larger_than_its_limit() {
    let apk = |files: &[(&str, Vec<u8>)]| {
        let copy = tempfile::NamedTempFile::new().unwrap();
        let mut archive = zip::ZipWriter::new(copy.reopen().unwrap());
        for (name, bytes) in files {
            archive.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap();
        copy
    };
    let paths = |names: &[&str]| names.iter().map(|name| name.to_string()).collect::<Vec<_>>();

    let copy = apk(&[("res/drawable/a.xml", b"small".to_vec())]);
    let path = copy.path().to_owned();
    let read = guest::read_apk(copy, &paths(&["res/drawable/a.xml", "res/drawable/b.xml"])).unwrap();
    assert_eq!(read, [Some(b"small".to_vec()), None]);
    assert!(!path.exists(), "the copy is left on the Host");

    let copy = apk(&[("res/drawable/a.xml", vec![0; 2 * 1024 * 1024])]);
    assert!(guest::read_apk(copy, &paths(&["res/drawable/a.xml"])).is_err(), "a 2 MB drawable was read");
}

/// Android's lookup of a resource tells its value and the APK it resolves
/// to, the last overlay that replaces it; an unknown resource does not
/// resolve. Planted defect this catches: the package's own APK taken where
/// an overlay replaces the resource. An emulator's output; milliseconds.
#[test]
fn a_resource_resolves_to_the_apk_the_device_takes_it_from() {
    let said = "Resolution for 0x0d030001 \n\
        \tFor config - en-rUS-port-420dpi-v36\n\
        \tFound initial: /system_ext/priv-app/SystemUIGoogle/SystemUIGoogle.apk #1\n\
        \tOverlaid: /data/app/~~G2q==/com.android.systemui.emulation.pixel_10-Bjg==/base.apk #13\n\
        Best matching is from default configuration of com.android.systemui.emulation.pixel_10\n\
        res/drawable/rounded_corner_top.xml\n";
    assert_eq!(
        guest::resolved(said),
        Some(Resolved {
            value: "res/drawable/rounded_corner_top.xml".into(),
            apk: Some("/data/app/~~G2q==/com.android.systemui.emulation.pixel_10-Bjg==/base.apk".into()),
        })
    );
    assert_eq!(guest::resolved("Error: failed to get the resource id\n"), None);
    assert_eq!(guest::resolved(""), None);
}
