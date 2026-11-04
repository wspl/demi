---
status: accepted
---

# A video plays with its own controls

## Problem

Alex opens a screen recording in the work panel. The web browser's own
controls show only while the pointer rests on the video, so Alex hunts for
them, and in a narrow panel they are drawn narrower than the picture, a dark
band across part of its width.

## Decision

**A video plays under a frosted capsule of Demi's own, always there, over the
bottom of the picture.** The picture keeps its size: the capsule floats on
it, centered, 12 px above its bottom edge, and takes no room of its own.

```text
+---------------------------------------------------+
|                                                   |
|                    the picture                    |
|                                                   |
|      ( > 0:03 ----o--------- 0:27  1x  <) [] )     |
+---------------------------------------------------+
```

- From left to right: Play or Pause, the time played, the playhead, the
  length, the playback speed, Mute and Full Screen.
- The capsule is the floating surface at about two thirds, blurred behind,
  with the floating shadow: white glass in a light theme, dark in a dark one.
  Its icons are 12 px in 20 px controls, its numbers 10.5 px and tabular; on a
  phone it is 40 px tall, with 16 px icons in 28 px controls, 12 px numbers,
  and a playhead a finger takes anywhere up and down the capsule.
- At most 360 px wide, and 24 px narrower than a smaller picture, where the
  playhead gives up the width and every button stays.
- The playhead is a 3 px line; its white knob shows while the pointer is on
  the capsule, a finger holds the playhead or it has the keyboard.
- A click on the picture plays or pauses. With the player or one of its
  controls focused, Space plays or pauses, but on a button, which it presses,
  and Left and Right go back or ahead 5 seconds; a viewer that steps between
  media with the arrow keys leaves them to the player.
- The speed goes round 1x, 1.5x and 2x.
- Full Screen takes the picture and the capsule to the whole screen, where an
  iPhone, which shows no element full screen, plays it in its own player.
- Every place that shows a video large uses it: the work panel, a file's
  preview and the phone's media viewer. An audio file keeps its player.

The design to build from:

- `web-ui` `files/VideoPlayer.vue` owns the video, its capsule and its
  source's release; `MediaPreview` sizes it as it sized the video, and keeps
  pausing it while it is hidden.
- `files/clock.ts` is the one clock text, which `AudioPlayer` and the
  capsule both show.
- The words of both players are one group, `player`, which `audioPlayer`
  becomes.
- `SwipeVideo` leaves a press on the capsule to it, by the capsule's own
  element, instead of a band of the video's bottom.

## Spike

None.

## Prior art

- [Vidstack](https://github.com/vidstack/player) and
  [Plyr](https://github.com/sampotts/plyr) draw their own controls over the
  video, show the container full screen, and fall back to
  `webkitEnterFullscreen` on an iPhone, as this does.
- Both hide their controls while the video plays and the pointer is still;
  Demi keeps them, since a recording is looked at closely, paused and moved
  through.

## Alternatives

- **The browser's own controls.** They are the problem above.
- **A bar under the picture.** It takes room from the picture's height.
- **Controls that hide while playing.** They are what made the controls hard
  to find.

## Open questions

None.
