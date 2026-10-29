---
status: accepted
---

# A viewer that falls behind holds no one else back

## Problem

Alex and a teammate watch the same simulator, each in their own page. The
teammate's page stops taking pictures: its tab is busy with something else, its
network drops for a while, or its laptop sleeps with the page still showing.
Alex taps Settings in the simulator and waits two to three seconds for the
screen to change, then waits again for the next tap. Nothing tells Alex why;
the simulator seems slow.

A source's viewers share one capture and one encoding, and the capture encodes
a new picture only while every viewer's window has room for it. A viewer whose
page stops acknowledging keeps its window full, so the capture waits for it,
and every other viewer waits too. The Host's log shows it as
`live_view_capture.unanswered` with `window` 0.

A page that is hidden, or whose panel no longer shows the source, already stops
watching and leaves the capture. What holds the others back is a page that is
still watching but no longer keeps up.

## Decision

A viewer that has held the capture back for longer than half a second, while
another viewer had room, falls behind. A viewer that falls behind gets no
picture and no longer paces the capture, so the others go on at their own pace.
Once its window has room again, it catches up: it starts over from a key frame,
as a viewer that just joined does.

```text
         window full while another has room, for HOLD
following ───────────────────────────────────────────▶ behind
    ▲                                                     │
    └──── its window has room: a restart and a key frame ─┘
```

- A source with one viewer never has a viewer behind: no one else waits for
  it.
- The viewer behind still steers the encoding (its bit rate, frame rate and
  scale), as before. A new scale starts a new capture for every viewer, and a
  path's capacity changes more slowly than a page stalls, so leaving the
  encoding to the pace alone would trade a stall for restarts.
- A page behind shows its last picture and its stall, as it does today when its
  pictures stop. When it catches up, its picture comes back from a key frame.
  The other viewers receive that key frame too, at most two a second, as for
  any viewer that asks.

### Design

The change is in `crates/live-view/src/stream.rs`, so a browser tab's viewers
get it as well as a device's.

- `Viewer` gains its following state:

  ```rust
  enum Following {
      /// Paces the capture; `held` is since when its window held it back
      /// while another viewer had room.
      Yes { held: Option<Instant> },
      /// Gets no frames and does not pace the capture until its window has
      /// room.
      Behind { since: Instant },
  }
  ```

- `allow` takes the least floor and window of the viewers that follow. The
  stream wakes at the earliest `held + HOLD` and moves a viewer still held to
  `Behind`.
- Frames go only to the viewers that follow.
- A viewer behind whose pace reports room gets the generation's `Restart` at
  the capture's newest sequence and asks the stream's `KeyFrames` for a key
  frame, the path a viewer that joins takes.
- The Host logs `live_view_viewer.behind` and `live_view_viewer.caught_up`
  with the time the viewer was behind, so a stall can be told from a slow
  source.

`HOLD` is half a second: the spacing the stream keeps between key frames, so a
viewer that falls behind again and again costs the others no more key frames
than the stream already allows.

The protocol, the page and the sources do not change. An older page behaves as
one that never falls behind.

## Spike

Whether a page that stops acknowledging slows another page's picture, and by
how much. Two pages watched one iPhone Duo on this Mac's runner. The second
page's script was paused with the debugger for 22 seconds while the first
tapped four rows of Settings, three seconds apart, recorded with
`demi browser record`.

| Tap | Second page running | Second page paused |
| --- | --- | --- |
| 1 | 0.2 s | 2.3 s |
| 2 | 0.2 s | 2.9 s |
| 3 | 0.2 s | in one late burst with tap 2 |
| 4 | 0.3 s | in one late burst with tap 2 |

The first page's picture came in bursts, two to three seconds apart, and the
Host logged `live_view_capture.unanswered` with `window` 0.

## Prior art

- LiveKit's stream allocator pauses a subscriber's track when its path cannot
  carry it, and its forwarder resumes the track at a key frame it asks the
  publisher for
  ([`streamallocator.go`](https://github.com/livekit/livekit/blob/master/pkg/sfu/streamallocator/streamallocator.go),
  [`forwarder.go`](https://github.com/livekit/livekit/blob/master/pkg/sfu/forwarder.go)).
  A paused subscriber does not slow the publisher or the other subscribers.
- mediasoup forwards a consumer's key frame request to the producer, and its
  `KeyFrameRequestManager` holds back requests that come too soon after one
  ([`KeyFrameRequestManager.cpp`](https://github.com/versatica/mediasoup/blob/v3/worker/src/RTC/KeyFrameRequestManager.cpp)).

Where this departs: LiveKit decides from its estimate of the subscriber's
bandwidth and keeps a track paused for at least five seconds. Demi's sender
waits for acknowledgements, so it decides from a window held full, and lets a
viewer catch up as soon as its window has room, since catching up costs one
key frame.

## Alternatives

- **Keep following the slowest viewer.** One page that stops stalls everyone,
  as the spike shows.
- **Drop frames for a slow viewer and keep it in the pace.** It still holds the
  capture while its window is full, and each dropped frame breaks the pictures
  after it until a key frame.
- **One encoding for each viewer.** Each viewer gets its own pace with no key
  frames for others, but the Host encodes the same picture once for each page,
  and a simulator's or a tab's capture is the Host's most costly work.
- **Simulcast, a few encodings at fixed qualities.** It serves viewers on paths
  of different capacity, which the shared encoding already handles; it does
  not help a page that stopped, and it multiplies the encoding.

## Open questions

None.
