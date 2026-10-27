---
status: proposed
---

# A foldable simulator folds and opens from its view and its command

## Problem

Alex builds an app for iPhone Duo and asks an agent to check its layout open
and closed. The iPhone Duo simulator boots closed, on its outer display, and
nothing in Demi opens it: `demi mobile` has no fold, the view has no
control, and only Xcode's Device Hub window on the Mac can move the hinge.
When Alex opens it there, the view's picture follows the inner display, but
taps still go to the outer one, and the screen's tree keeps the outer
display's size, so the agent taps the wrong places.

The same holds for an Android foldable such as Pixel 10 Pro Fold: the
emulator folds from Android Studio's toolbar and its console, neither of
which Demi calls, and the view keeps the open device's frame and size
whatever the emulator shows.

## Decision

A foldable device folds and opens from its view and from `demi mobile fold`.
iPhone Duo shows as Apple's own 3D model of it, with a hinge the user sets to
any angle; an Android foldable shows as its 2D frame, open or closed.

### What the user sees: iPhone Duo

```text
+--------------------------------------------------------------+
| iPhone Duo (iPhone Duo)   [rotate] [home] [lock] ...         |
+--------------------------------------------------------------+
|                                                              |
|        .-----------------.  .-----------------.              |
|       /   left page     /  /   right page     /               |
|      /  (inner display, one picture across)  /                |
|     '-----------------'  '-----------------'                 |
|                                                              |
+--------------------------------------------------------------+
| [Closed] [Half Open] [Open]  o-----------*----- 127°  [Reset] |
+--------------------------------------------------------------+
```

- The view is for working the device, not for showing it off: the model
  says how far the device is open and which display is lit, and nothing
  more. The device is Xcode's 3D model of iPhone Duo in the model's own
  finish, with the live picture on the display that is lit: the outer
  display while closed, the inner one open and half open, as the simulator
  lights them.
- The toolbar holds Closed, Half Open and Open among the device's other
  controls, the posture the device is at pressed. A posture moves the hinge
  to 0°, 130° or 180° over about 0.6 s, as Device Hub does, and the model
  folds with it; its button pressed again moves the hinge there again,
  which a fold outside Demi may have left elsewhere.
- A capsule at the bottom of the view, on the view's own background, holds
  a hinge slider from 0° to 180° with the angle beside it, and Reset. The
  slider moves the hinge as it is dragged, one fold at a time, to the
  latest angle. A fold the device refuses leaves the view at the angle the
  device reports. The capsule keeps its place in
  every posture, so nothing around it moves.
- Half open shows the device from between its two pages, so both halves of
  the inner display face the user and both take taps.
- While the device is partly open, dragging its body, a dark display, the half
  in front of the lit one or the space around it turns the view about the
  device's vertical axis, at most 60° either way, where it stops. Closed or open
  flat, the device faces the user and the view does not turn: the turn eases out
  as the hinge nears 0° or 180°. Reset faces the device again; it is dimmed
  while the device faces the user. A click there does nothing.
- On the lit display a click is a tap and a drag a swipe, as today, in every
  posture and view: the point is found on the display as the user sees it,
  turned or folded.
- Every other control of the view (rotate, Home, Lock, the volume keys,
  appearance, screenshot, record) works as it does for an iPhone. A device
  held sideways shows turned about the view's axis, as a 2D frame turns,
  with its picture upright.
- The view says what it waits for, a picture or a frame that cannot show,
  and a stream that reconnects or downloads, as every device's view does.

### What the user sees: an Android foldable

- The device shows in its 2D frame around the display that is lit: the art
  Android Studio installs for its model, open or closed, as Android Studio
  draws it, or the drawn frame where none is installed.
- The toolbar holds Closed and Open, with the names and icons iPhone Duo's
  have, which fold the device closed or open it flat; the view shows the
  other display once the emulator switched.

### What the agent sees

```text
$ demi mobile open alex-duo
m1  Alex's Duo · iOS 27.1 · 1078×1568 · closed at 0° · simctl id …
$ demi mobile fold m1 half-open
m1 is half open at 130°: the inner display shows, 1568×1103.
$ demi mobile fold m1 100
m1 is half open at 100°: the inner display shows, 1568×1103.
$ demi mobile inspect m1
Device: m1 · Alex's Duo · portrait · 1568×1103 · half open at 100°
…
```

- `demi mobile fold <device> closed|half-open|open|<degrees>`. An Android
  foldable takes `closed` and `open` only. On any other device the command
  fails, saying the device does not fold. It answers once the device shows
  the display the fold lights.
- `open` and the header of `inspect` say a foldable's posture. `list` names
  devices that are not open too, whose posture Demi does not read, so it
  says none.
- After a fold, `screenshot`, `inspect`, `tap`, `swipe` and `type` act on
  the display that is lit, in its size; a ref listed on the other display,
  whoever folded the device, is no longer valid, and the action says to
  inspect again.

### Design

- **The iOS helper** (`crates/plugin-mobile/command/ios-sim`):
  - Moves the hinge with the vendor-defined HID event Device Hub sends:
    `IndigoVendorDefinedEvent` to dtuhidd's
    `com.apple.coredevice.feature.remote.hid.vendordefined` feature, over the
    XPC endpoint `-[SimDevice lookup:error:]` answers for
    `com.apple.coredevice.feature.remote.hid.digitizer`, with usage page
    `0xFF61`, usage `0x5B`, version 1 and IOKit's compact XML of
    `{source: hinge-slider-control, type: range, value: <whole degrees>}`.
    A posture sends about 35 angles over 0.6 s, easing out.
  - Reads the displays from CoreSimulator's display configurations,: a
    device folds when it has two integrated displays, the outer
    being the primary one. The configurations name the lit display, the
    guest's own choice, which lags the hinge: an open sweep lights the
    inner display past about 90°, a closing one lights the outer below
    about 20°, and small steps keep the outer lit up to about 120°.
  - Attaches the framebuffer to the lit display, and again when the device
    lights the other, whoever folded it. HID and the accessibility tree
    follow the active display. The accessibility tree of the home screen
    on the inner display reads in its unturned points, an app's upright,
    which the helper tells apart by whether an element reaches past the
    unturned width.
  - Answers a fold once the lit display is the one the angle lights, and
    half a second later: a fold sent at once after the guest switched
    displays leaves it switching until it is locked and woken. A guest
    that keeps its display for 6 s kept it, and the answer names it.
  - Knows the angle of its own folds only: no call reads the hinge, so a
    fold outside Demi, as from Device Hub, reads by the display it lit,
    open flat for the inner display and closed for the outer.
  - Reports the path of the device's 3D model, from its device type's
    product class and the selected Xcode's DeviceKit.
- **The Android driver** folds with the emulator's gRPC `setPosture`, and
  reads the posture from the region of the panel each picture of its
  stream shows, `foldedDisplay`, whoever folded it. The emulator's display
  0 shows the lit display, and the guest's display 0 with it, so
  screenshots, the tree and touches need no display ID; touches are placed
  on the folded region's pixels. A device is foldable when its AVD names a
  hinge's closed region. Its frame is Android Studio's art of the model's
  `default` or `closed` folder.
- **The protocol** (`crates/plugin-mobile/protocol`):

  ```rust
  /// How a device folds, and how it is folded now.
  pub enum LiveFold {
      /// iPhone Duo: a hinge from 0° (closed) to 180° (flat), the lit
      /// display, each display's size, and the path of the 3D model.
      Hinge {
          degrees: u16,
          display: FoldDisplay,
          outer: LivePoints,
          inner: LivePoints,
          body: Option<String>,
      },
      /// An Android foldable: open or closed.
      Twofold { open: bool },
  }
  ```

  `LiveDevice` gains `fold: Option<LiveFold>`. The page reads the model's
  file (`SharedFrameworks/DeviceKit.framework/…/<productClass>.usdz`) the
  way it reads any file of the Host, by its path, so the stream carries no
  bytes of it. `fold` is an operation, as `rotate` is, and the view's
  controls call it. A frame the page keeps is a display's, so a foldable
  that lit its other display shows in that display's frame.
- **The page** (`crates/plugin-mobile/page`) loads three.js, and its USD loader
  for a model, only when a device folds on a hinge. It reads the model's layer,
  in its own finish with no color variant, lights it with a room environment, so
  its metal does not show black, and shows the video on the mesh of the display
  the Host names lit. It finds the two displays by their shape, since the file's
  names are scrambled, and the stretch of the animation that folds the device by
  the file's SHA-256, as Device Hub Pro pins it; a file of another hash is used
  on the same numbers when its animation is as long. A pointer is cast onto the
  device and read as a point of the screen where it meets the lit display first.
  Where Xcode has no model, or its file does not fit, the view draws a stand-in
  of two slabs and a hinge, sized from the two displays; where WebGL 2 is
  missing, the 2D frame of the lit display, with the postures in the toolbar.
- **The gallery** shows iPhone Duo on the stand-in, since Apple's file is
  never committed, with the postures, the capsule's slider and the view's
  turn, and an Android foldable's two frames. Its controls fold the devices
  as the product would.

## Spike

On this Mac, with Xcode 27.1 (27A9275) and iOS 27.1 (24A94425):

- A probe sent the hinge event from 0° to 180° to a booted iPhone Duo: the
  inner display lit with the home screen, 2853×2007 pixels; 180° to 0° lit
  the outer display again.
- A device `demi mobile` opened open went on showing the lit display after
  a fold, but its taps stayed on the display it was opened on: Settings did
  not open from the outer display until the device was opened again. Its
  tree kept the inner display's size.
- three.js 0.186's `USDLoader` read Xcode's `V68.usdz` (5.9 MB) in under
  100 ms once given its model layer, since it reads neither the root
  layer's payload nor its variant: 73 meshes, a skeleton and one animation
  of 37.5 s, whose stretch from 5.4167 s (flat) to 10.4167 s (closed) folds
  the device linearly. With the color variant's 37 opinions applied and a
  room environment, both finishes rendered; the two displays took a
  screenshot each as a texture, upright, and half open from between the
  pages both halves read.
- The Android emulator 36.6, with Pixel 10 Pro Fold on Android 36, folded
  and opened from `adb emu posture` and `fold`/`unfold`: the display
  switched between 2076×2152 and 1080×2364, and `screencap` warned that it
  picked the first of two displays. The emulator's gRPC `setPosture` did
  the same, its display 0 showing the lit display, and its picture stream
  named the folded region.

## Prior art

- Xcode 27.1's Device Hub folds iPhone Duo with Closed, Half Open and Open
  and a hinge slider, and draws the device as this model.
- [Device Hub Pro](https://github.com/gitemre/device-hub-pro) (MIT) found
  the hinge event and the lit display's screen ID, and draws Xcode's model
  read at runtime, never copied. Demi does the same in the web page, for
  the use Xcode's own Device Hub makes of it: developing an app on the
  user's own simulator. The model is never committed or stored beyond the
  session.
- Android Studio's emulator folds from its toolbar and from
  `adb emu fold`, `unfold` and `posture`.
- [auto-mobile](https://github.com/kaeawc/auto-mobile) gives agents a
  `setPosture` tool; on iOS it moves the hinge from a runner inside the
  simulator, which Demi has no need for.
- macOS Quick Look turns a USDZ model by dragging it, with no other control.

## Alternatives

- A 3D model Demi draws for Android foldables: it would look like no real
  device, where Android Studio's frames look like the one simulated, and
  with only Closed and Open there is no angle to show.
- Half Open pressed again to face the other page: the same button doing a
  second thing is found by chance, and from between the pages both are
  read.
- A control of its own to turn the view: dragging the device or the space
  around it does the same with nothing more on screen.
- Turning the view freely, or about any axis: past 60° the display is too
  slanted to read or tap, and a device seen from the side or behind tells
  nothing about the app.

## Open questions

None. The display configurations CoreSimulator sends tell the helper that
someone else folded the device.
