---
status: accepted
---

# A turn shows as the device turning

## Problem

Alex presses Rotate Right on an Android emulator in the work panel. The
device's frame snaps to landscape at once, and for 0.3 to 0.8 seconds the
screen inside it shows the app lying on its side, until Android lays it out
again. On a phone in the hand the same delay goes unseen: the hand takes
about as long to turn the phone, and Android catches up as the turn ends.
Android Studio's emulator window snaps the same way.

The delay is Android's own. The emulator turns its pictures as soon as its
sensors say the device turned, and Android waits for the sensor's reading to
settle before it turns its windows, a reading Demi sets as Android Studio
does. Turning the sensor gradually, as Android Studio's default does, makes
the wait longer: about 1.6 seconds.

## Decision

A device that changes the orientation it shows turns there in 0.3 seconds,
about its center, from where it was, as a phone turned in the hand, and as
the iOS Simulator turns its window:

```text
 ┌──┐        ╱╲          ┌─────┐
 │  │  ──▶  ╱  ╲   ──▶   │     │
 │  │       ╲  ╱         └─────┘
 └──┘        ╲╱
 0 s        0.15 s        0.3 s
```

- It applies to every turn the view shows: one the viewer presses, one the
  agent makes (`demi mobile rotate`), and one the device makes itself.
- The device turns the shorter way, and grows or shrinks during the turn
  to the size its new orientation fits the panel at.
- The screen shows the device's pictures throughout. A picture that is
  still sideways when the turn ends stays until Android turns it, as now.
- With Reduce Motion on, the device turns at once, as now.

### Design

`crates/plugin-mobile/page/src/live/DeviceView.vue` wraps the device in a
box it animates with the Web Animations API when the orientation the view
shows changes. `view.ts` holds the geometry:

- A turn's pose is its angle, scale and offset from where the new layout
  puts the device, about its screen's center. It starts as the pose that
  shows the new layout where the device showed, and eases to none.
- A turn that starts during another, or a layout that changes during one,
  as when the new orientation's frame arrives, starts from the pose the
  device shows then, so the device never jumps.
- At every step the pose is scaled down where the turned device would
  leave the area, which a square area needs at 45 degrees.
- A touch, a drag or a wheel turn during a turn lands on the point of the
  screen the turning device shows under the pointer.

Nothing else changes: the layout, the input and the session see the new
orientation at once.

## Spike

1. **Where the delay is.** A recording of the page at its frame rate shows
   the frame turning 13 to 133 milliseconds after the press, and the
   picture's content turning 0.28 seconds later on one turn and 0.78
   seconds on another.
2. **Whether the sensor's interpolation shortens it.** Turning the sensor
   smoothly made the content turn 1.6 seconds after the frame.

## Prior art

- The iOS Simulator animates its window's turn, and iOS turns the content
  with it.
- Android Studio's emulator window turns at once on the emulator's answer
  (`EmulatorRotateAction.kt`, `EmulatorView.kt`), and its content waits for
  Android as Demi's does.

Where this departs: Android Studio does not animate; Demi does, as the iOS
Simulator does, since a turn without motion shows Android's wait.

## Alternatives

- **Keep the snap**, as Android Studio does: the sideways screen shows for
  up to 0.8 seconds after each turn.
- **Hold the device until Android turns**, up to a second: no sideways
  screen, but every press answers 0.3 to 0.8 seconds late, and an app that
  stays portrait turns only at the limit.

## Open questions

None.
