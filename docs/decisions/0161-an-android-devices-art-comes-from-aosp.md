---
status: accepted
---

# An Android device's art comes from AOSP, and its screen takes the device's shape

## Problem

Alex creates a Pixel 10 virtual device with the SDK's command-line tools and
has never installed Android Studio. Demi opens it, and the work panel shows a
plain rectangle with square corners around a square screen, nothing like a
Pixel 10. Two things are missing, and both come from Android Studio today:

- **The frame.** Demi draws a device's frame from the device art in Android
  Studio's installation (`plugins/android/resources/device-art-resources`).
  Without Android Studio there is none, and the view draws its stand-in frame.
- **The screen's shape.** The system image holds overlays for each Pixel,
  such as `com.android.internal.emulation.pixel_10`, which give the screen
  its rounded corners and camera cutout. The emulator turns them on only as
  a cold boot completes with the device's skin, Android Studio's device art
  again. A virtual device that `avdmanager` creates has no skin, so they stay
  off, and so does one started from a snapshot saved while they were off.
  Its screen stays square, and the stand-in's corners, which follow the
  screen's, are square too.

## Decision

Demi takes every Android device's art from the Android Open Source Project
(AOSP), where Android Studio's comes from, and never from Android Studio: one
source, the same on every Host whatever is installed on it. And Demi turns on
the device's overlays itself, so its screen has the device's shape however the
virtual device was created or started.

- **Where.** The art lives in AOSP's `platform/tools/adt/idea` repository, in
  `artwork/resources/device-art-resources`, one folder per device, such as
  `pixel_10`, and one folder per posture for a foldable, under the Apache
  License 2.0. Demi reads it at one commit it pins.
- **When.** The first time a Host opens a virtual device whose
  `hw.device.name` names a device with art, the Host downloads that device's
  art and keeps it for every later use. A device with no art at AOSP draws the
  stand-in, as now. A Host that cannot reach AOSP draws the stand-in too,
  says why in its log, and tries again the next time the device opens.
- **Checked.** Demi carries the SHA-256 of every file it may download, as it
  does for its other downloads, and keeps a device's art only when all its
  files match.
- **The screen's shape.** Once the emulator booted, Demi turns on the system
  image's overlays named for the device, the ones the emulator turns on after
  a cold boot with the device's skin. A virtual device that has them on is
  left as it is.
- **A frame a page kept.** The page keeps each device's frame across reloads
  and shows it at once, but asks the Host for the frame again, whose answer
  replaces it, as the plugins' cache promises. A page that kept the stand-in
  shows the device's own frame as soon as the Host draws it.

The code that finds Android Studio's installations is removed.

### Design

- `crates/plugin-mobile/command/src/android/art.rs` keeps the skin's parser
  and drawing. In place of the installations, a pin names the commit and,
  for each device, its files and their SHA-256 (`art_pin.rs`). The
  `xtask device-art` command writes the pin from the head of Android
  Studio's main branch, or a commit it is given, so a new Pixel's art comes
  with a new pin.
- A device's folder is downloaded from Gitiles at the pinned commit as one
  archive (`+archive/<commit>/<path>.tar.gz`): Gitiles refuses a client that
  asks for many files in a row. Its files go into
  `<installation>/mobile/android/device-art/<commit>/<device>/` through a
  folder beside it, moved into place once every file matches.
- `Guest::emulate_device` lists the overlays (`cmd overlay list`) and turns
  on, with `cmd overlay enable-exclusive --category`, each one that is off
  and whose package ends in `.<device>`, as the emulator's `QemuMiscPipe`
  does. The driver calls it as it attaches the device.
- The mobile page's session asks the Host for a frame it showed from its
  disk cache.
- An older runner draws as before. The overlays are a setting the virtual
  device keeps, as the emulator's own would be.

## Spike

1. **Whether the screen depends on the skin.** A Pixel 10 virtual device
   created with `avdmanager` and started without a skin reports no rounded
   corners and no cutout, and its Pixel 10 overlays are off. The same system
   image started cold with `-skindir` and `-skin pixel_10`, the folder
   downloaded from AOSP, turns on `com.android.internal.emulation.pixel_10`
   and `com.android.systemui.emulation.pixel_10`, and reports corners of 132
   pixels and a cutout.
2. **Why not start with the skin.** The same skin on a virtual device that
   starts from its snapshot turns nothing on: the emulator only does so after
   a cold boot. A foldable's folder has no layout of its own, only one per
   posture: given the folder, the emulator falls back to a screen of 320 by
   480 pixels; given the open posture's, it reports its pictures turned a
   quarter while the device is closed.
3. **Whether turning the overlays on in a running emulator is enough.**
   Turned on with `cmd overlay enable-exclusive`, the Pixel 10 reports
   corners of 132 pixels and a cutout at once, and a Pixel 9 Pro Fold
   corners of 85 pixels, open and closed, with no restart.
4. **Whether AOSP serves the art.** Gitiles lists 68 entries, 61 of them
   devices with a layout, among them `pixel_2` to `pixel_10`, the Pro, XL,
   `a` and Fold models, the tablets and Wear OS. A phone's three files are
   about 230 KB, a foldable's two postures about 780 KB. A run of single
   files is refused with 429 after about thirty requests; one archive per
   device is not.

## Prior art

- Android Studio's emulator window draws the frame from these files
  (`SkinDefinition` in its `adt-ui`), and Android Studio sets a new virtual
  device's skin to the device's folder.
- The emulator's `QemuMiscPipe` turns on the overlays whose names end in the
  skin's name after a cold boot.

Where this departs: Android Studio ships the art in its installation; Demi
downloads one device's art when it is first needed. The emulator turns the
overlays on only after a cold boot with the skin; Demi turns them on after
any boot, so a snapshot or a virtual device created without the skin gets
them too.

## Alternatives

- **Android Studio's art where it is installed, AOSP's elsewhere.** Two
  sources for one thing, and a frame that differs by what a Host has
  installed.
- **Start the emulator with the art as its skin.** It covers only a cold
  boot of a phone, as the spike shows, and breaks a foldable's screen.
- **Demi ships all the art.** Several megabytes in every runner's command
  package, for devices most Hosts never open.

## Open questions

None.
