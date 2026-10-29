---
status: proposed
---

# A device turns only when it is rotated

Supersedes:

- 0133 (A foldable simulator folds and opens), the part that a device held
  sideways shows with its picture upright.
- 0165 (A turn shows as the device turning), the part that a turn the device
  makes itself turns the view.

## Problem

Alex opens iPhone Duo flat in the work panel and presses Rotate Right: the
inner display turns its home screen, and the device shows turned, its hinge
across. Alex drags the hinge slider towards closed. Near 10°, the guest
lights the outer display, whose home screen does not turn, and the device
turns back a quarter on its own. Alex drags it open again: the device stays
upright until 180°, where the guest lights the inner display again, and the
device turns a quarter once more. Dragging to and fro across the angles
where the guest switches displays turns the device back and forth, its
pictures with it.

An iPhone does the same without folding. Alex holds it upright and opens a
game that shows only in landscape: the device in the view turns a quarter on
its own. Alex closes the game, and the device turns back.

The view turns an iOS device as far as its screen's picture is turned, which
the app on the screen decides, and for iPhone Duo the display the guest
lights. An Android emulator's view already turns only when it is rotated:
its pictures follow the device's sensors, and an app that does not turn is
drawn on its side.

## Decision

A device in the view turns only when it is rotated: by Rotate Left or Right
in the toolbar, or by `demi mobile rotate`. An app that turns or refuses to,
and folding, opening and the guest's switch between its displays, never turn
it.

- The device shows held the way it was last rotated, as Simulator.app,
  Xcode's Device Hub and Android Studio hold it: upright once Demi opened it,
  as Demi turns it upright then.
- The screen shows its picture as the app lays it out. An app that does not
  follow the device lies on its side in the screen: a game in landscape on an
  iPhone held upright, or the outer display's home screen of iPhone Duo held
  sideways, as on the phone in the hand.
- A click and a drag land where the user sees them, on the picture as it
  lies; the agent's screenshots, coordinates and screen tree stay as the app
  lays the screen out.
- A rotation turns the device as 0165 animates it, and the picture inside
  turns once the app turns it.

```text
 iPhone held upright,           iPhone Duo held sideways,
 a game in landscape            closed after it was flat
 ┌───────┐                      ┌─────────────┐
 │ ┌───┐ │                      │ ┌─────────┐ │
 │ │ ↻ │ │  the game on its     │ │ home ↻  │ │  the home screen
 │ │   │ │  side; the device    │ │ screen  │ │  on its side; the
 │ └───┘ │  stays upright       │ └─────────┘ │  device stays turned
 └───────┘                      └─────────────┘
```

### Design

The live view shows the device as it is held; everything else stays the
screen's. The page does not change: it already turns the device by the
orientation of its pictures and frame, which become the held one.

- The iOS helper (`DeviceSession.swift`) owns how the device is held: upright
  once attached, and the orientation of each rotation. It tells it with the
  screen, and `demi-mobile` reads it there (`Screen::held`); an Android
  device is held as its screen shows, since its pictures follow its sensors.
- The helper's live captures turn each display's picture upright as the
  device is held, not as its screen shows, and it draws the frame that way.
  Screenshots, recordings, the screen's tree and the agent's coordinates
  stay the screen's.
- `demi-mobile`'s live view names the held orientation, with the screen's
  points turned to match, in its pictures' layout, its frame and the
  device's state (`Screen::as_held`), and turns each point a viewer gives
  back into the screen's before it reaches the device (`Screen::from_held`).
- `rotate` answers how the device is held after the turn (`RotateResult::held`)
  beside the screen's orientation, which the page's press waits for.

The page and `demi-mobile` ship in the same release.

## Spike

A trace of the page's state while dragging the slider of iPhone Duo held
sideways: from 36° to 180° the outer display was lit, in portrait, and the
inner display was in landscape; the device turned a quarter at 180°, when the
inner display lit.

## Prior art

- Simulator.app and Xcode's Device Hub turn the device only on a rotation,
  and show an app that does not follow it on its side.
- Android Studio's emulator turns its frame only on a rotation
  (`EmulatorRotateAction.kt`).

## Alternatives

- **Animate the turns that follow the screen**, as 0165 animates them: the
  device still turns on its own when it folds or an app turns.
- **Turn iPhone Duo by its inner display's orientation always**: it still
  turns on its own when an app on the inner display turns or refuses to, and
  an iPhone keeps turning on its own.

## Open questions

None.
