---
status: proposed
---

# Each display of a foldable shows live on its own face, and the hinge moves as Device Hub moves it

Supersedes: 0133 (A foldable simulator folds and opens from its view and its
command), the parts that say the view shows one live picture, on the display
the guest settled on; that a posture sends about 35 whole-degree angles over
0.6 s; and that the slider moves the hinge one fold at a time.

## Problem

Alex works on an app on iPhone Duo and presses Closed. Within half a second
the simulator's outer display shows the home screen, but the view shows the
closed device with a black display for about 3.5 seconds more: the view
shows one display at a time, and changes to the other only once the guest
says its switch is over. Opening is black for about 0.3 seconds. iOS's own
animation of the switch, which runs on the display that lights, never shows.

Dragging the slider queues the folds: each waits for the one before to end,
so the hinge goes on moving for seconds after Alex lets go.

## Decision

### What the user sees

- Each display of the device shows its own live picture on its own face of
  the model: the outer display on the cover, the inner display across the
  two pages. A display the guest turns off shows black, as the real device's
  does, and a face turned away from the user is not drawn. On a fold, the
  display that lights shows its picture the moment the guest draws it, with
  iOS's own animation of the switch, as in Xcode's Device Hub.
- Clicks, the keyboard, the screen's tree and the agent's screenshots go to
  the display the guest lights, from the moment it names it: about half a
  second into a close, not three and a half.
- A posture moves the hinge smoothly to its angle; a posture pressed while the
  hinge moves turns it towards the new angle from where it is, without
  stopping. The slider moves the hinge as it is dragged, towards the latest
  angle, and the hinge stops where the slider is let go.
- The model follows the hinge with Device Hub's spring, so it never jumps.
- Nothing else changes: the toolbar, the capsule, Reset, turning the view and
  `demi mobile fold` stay as 0133 has them.

### The design

- **Two captures.** The helper attaches the framebuffer of each of a
  foldable's two integrated displays, as Device Hub's two display views do,
  and encodes each for its own capture. Each display is a source of the
  shared live view, with its own stream, delivery and rate control, as two
  browser tabs are. A display that is off shows a still black picture, which
  its capture sends next to nothing for once it is coded. A device that does not fold keeps its one capture.

  A view's stream names the display it carries in its hello; a stream that
  names none carries the lit display, as every stream does today.

  ```rust
  LiveViewerMessage::Hello {
      platform: Platform,
      device: DeviceId,
      /// The foldable's display whose pictures the stream carries; none
      /// for the display the guest lights.
      display: Option<FoldDisplay>,
  }
  ```

  The sources of `m1`'s displays are `m1.outer` and `m1.inner`; `m1` stays
  the lit display's. The 3D view opens a stream for each display and draws
  each on its face, at the size that face shows; its device's own stream
  carries the state and the input, and no pictures, which the view says
  (`LiveViewerMessage::Unshown {}`) so the Host captures none for it.
- **The display in focus.** The guest aims touches at the display it
  lights, and Demi has no call that aims them elsewhere, so the display in
  focus is the guest's: the one its display configuration names incoming,
  from then on, and the active one once the configuration settles. The tree,
  screenshots and the points of input are that display's, and the view's
  state names it. Device Hub picks its display by the hinge's angle instead
  (the cover while closed and at 45° or below, the inner display at 55° or
  above), since it reads the hinge as it moves and aims its touches at the
  display it picked.
- **The hinge.** The helper sends the angle every 16 ms on a critically damped
  spring with a smoothing time of 0.2 s, until it is within 0.25° of the
  target and slower than 3° a second; a new target turns it from where it is,
  at the speed it has. Each event is the angle as a real number, serialized
  by IOKit in its binary form, with version 0 and Device Hub's provider, as
  Device Hub sends it. The view's postures and slider send the angle on the
  view's stream (`LiveViewerMessage::Hinge { degrees }`), which turns the
  hinge towards the latest angle at once, in the order they came, and
  answers nothing; `demi mobile fold` waits, as
  0133 says, until the display the angle lights is the guest's.
- **The model.** The page moves the model towards the hinge's angle on a
  spring with a smoothing time of 0.25 s, settled within 0.05°.
- An older page opens `m1` alone and shows the lit display, as before. An
  older runner refuses a hello that names a display, so its 3D view shows no
  picture until the runner updates; nothing stands in for it.

## Spike

On this Mac, with Xcode 27.1 and iOS 27.1, on an iPhone Duo created for the
check:

- Closing from open 6 times, sampling the view every 300 ms and the
  simulator's own screenshots of both displays: the outer display showed the
  home screen within 0.3 s of the click; the display configuration named it
  incoming at 0.5 s and settled at 3.6 s; the view was black from about
  0.6 s to 3.6 s. Opening: incoming at 0.3 s, settled at 0.6 s.
- Device Hub's binaries, disassembled: the iPhone Duo plug-in makes two
  display views, one per display, each a SimulatorKit view connected to its
  display's screen for rendering and properties, and shows or hides each by
  whether its face looks at the camera. The rule of focus, the hinge's
  spring, its event and the model's spring are as the design says. Its
  snapshot blur covers a window's resize, not a fold, and its 60 s timeout
  is a screen connection's.
- A helper sending Device Hub's event (a real number, binary, version 0) on
  its spring folded the device closed, half open and open, as the whole
  degrees in XML do.
- Open, then Closed queued behind it, 15 times with each event: neither left
  the guest switching. Earlier, 3 folds in about 60 left it switching until
  it was locked and woken; what starts it is not known yet, and this
  decision does not change it.

## Prior art

- Xcode 27.1's Device Hub, as the Spike reads it from its binaries.
- [Device Hub Pro](https://github.com/gitemre/device-hub-pro) (MIT) shows
  one panel at a time and reads the lit one from the two displays' pictures,
  the one that is not black. Demi shows both, as Device Hub does, so no
  display needs to be guessed for the picture; the focus still does.

## Alternatives

- One capture that moves to the display the guest names incoming: black for
  half a second on each fold, and iOS's animation lost on the display Demi
  had not moved to yet.
- A blurred snapshot over the hand-over: Device Hub has none for a fold.
- Composing both displays into one picture on the Host: the two displays
  differ in size and turn, and the page would cut them apart again.

## Open questions

None.
