---
status: accepted
---

# A video's volume is set above Mute

Supersedes: 0253 (A video plays with its own controls), the list of the
capsule's controls, which gains the volume.

## Problem

Alex plays a screen recording with a loud soundtrack in the work panel. The
capsule offers Mute and nothing between it and full volume, so Alex turns
down the whole computer, and every other sound with it.

## Decision

**With a pointer that hovers, a volume slider stands above Mute.** It floats
over the picture, 8 px above the capsule, while the pointer is on Mute or on
the slider, or the keyboard is on either; the capsule's layout never moves.

```text
                                   ( | )
                                   ( o )
                                   ( | )
  ( > 0:03 ----o--------- 0:27  1x  <)  [] )
```

- A frosted panel 32 px wide and 112 px tall, as the capsule is drawn, with
  an 80 px vertical track and the white knob of the playhead.
- Silence is Mute: the slider at the bottom mutes, and any volume above it
  unmutes. Mute shows the slider at the bottom; unmuting a video muted by
  the slider brings back the volume kept.
- The browser keeps the last volume above silence, and every video starts at
  it. Mute is not kept.
- Mute shows a speaker with one wave below half the volume, and two above.
- With the keyboard, Tab goes from Mute to the slider, whose arrows move the
  volume and leave the playhead where it is. A click on Mute does not keep
  the slider open once the pointer leaves.
- A device without a hovering pointer, a phone or a tablet, has Mute alone:
  an iPhone's page cannot set a video's volume, and its buttons do.

The design to build from:

- `VideoPlayer.vue` holds the slider beside Mute, inside the capsule's
  element, so `SwipeVideo` and the media viewer leave a press on it to the
  player as they do one on the capsule. The capsule draws its blur on a
  layer of its own, so the panel's blur sees the picture.
- The volume is kept in `localStorage` as `demi-video-volume`.

## Prior art

Vimeo, QuickTime Player and Safari's own video controls show the volume
above the speaker while the pointer rests on it. YouTube widens the capsule
instead, which moves the times beside it. YouTube and Vimeo keep the volume
for the next video; their phone players have no slider.

## Alternatives

- A short slider beside Mute, always there: the playhead loses 64 px, most in
  a narrow panel.
- A horizontal slider above Mute: a wider panel, further from the speaker.
