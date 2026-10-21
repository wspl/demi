---
status: accepted
---

# A recording keeps every drawn frame, from the picture the view streams

## Context and Problem Statement

An agent checks what a screen does over time, such as a rotation, a tab
switch or an animation, with single screenshots. A defect that lasts one or
two frames, a frame that jumps or a picture that blinks, falls between them.
`demi mobile record` runs each system's own recorder and writes an MP4 a
model cannot read, and `demi browser` cannot record at all.

The work panel already captures each device's and each tab's picture on the
Host and streams it. That capture takes pictures at the stream's rate, which
falls with the network, so a frame that shows for 16 ms can be skipped.

## Precedents

* scrcpy encodes an Android screen as H.264 with the device's own
  hardware encoder and records that stream, frames dropped by the encoder
  included, without counting them.
* Playwright's `recordVideo` takes Chrome's screencast, a JPEG per frame
  at the screencast's rate, and encodes it with ffmpeg as VP8; frames the
  screencast skips are not known.
* idb's `record-video` and `simctl io recordVideo` write the Simulator's
  frames as H.264 with VideoToolbox.

Demi departs from them in three ways: the stream and the recording share
one capture, a frame identical to the last is not kept, and a frame the
capture or the encoder could not take is counted and placed in the index.

## Considered Options

* Keep the systems' recorders and decode their MP4s into frames.
* Keep the frames as lossless images, only those that changed.
* Capture every drawn frame once, for the stream and the recording alike,
  and keep a recording as a video of the frames that changed, with an index
  of where each one changed.

## Decision Outcome

Chosen option: one capture and a video, because the stream and the
recording are one picture used two ways, and a video is an order of
magnitude smaller than lossless frames when the whole screen moves, while
it keeps what a defect is: a frame that differs.

```
                      +-> stream: the newest frame, encoded at the
Host captures each    |   panel's size, at the network's rate
drawn frame of a   ---+   (dropped when behind)
device or a tab       +-> recording: every frame that differs
                          from the last, encoded at the screen's
                          own size with its time, and where it
                          changed, in an index
```

* **One capture.** A device or a tab has one capture on the Host. It runs
  while the view is looked at or a recording runs, and takes each frame the
  screen draws when the source tells it drew one, rather than ticking at a
  rate:
  * Chrome: the tab capture the live view already uses, read frame by frame
    in the browser, where a recording encodes every frame with the browser's
    own encoder, the hardware's where it has one. Measured at 60 Hz, it gave
    every frame of a busy page for a few percent of a core, where Chrome's
    screencast gave every frame too but cost ten times as much, most of it
    in compressing each frame to a JPEG.
  * iOS Simulator: its frame callback, the frame copied at once into
    reused memory.
  * Android emulator: its screenshot stream at the screen's own pixels,
    through shared memory.

  The stream takes the newest frame when it can send one; a recording takes
  them all. Both are encoded as H.264 by the Host's hardware encoder, the
  stream at the panel's size and a recording at the screen's own size:
  VideoToolbox on macOS and Media Foundation on Windows. Linux, and a
  Host whose hardware encoder fails to start, encode in software, and an
  Android recording there is kept at the device's logical size, the most
  software keeps up with.
* **What is kept.** A frame identical to the one before is not encoded. The
  others are encoded at a quality that keeps a one-pixel move visible, with each frame's own time,
  so a still screen costs nothing. Beside the video, an index says for each
  frame its number, its time and the area that changed. A frame the capture
  could not take, or the encoder dropped, is counted, and the index marks
  where it fell. A recording keeps the screen's own size while a view of
  it is looked at; the view is scaled from it.
* **Commands.** `demi browser` and `demi mobile` record the same way:
  * `record <tab or device> start [--duration <time>]` answers a recording
    id at once. A recording with a duration stops itself when it is over;
    one without runs until it is stopped or reaches the limit below.
  * `record <id> stop` stops a recording, or answers for one that already
    stopped itself, and says how long it ran, why it stopped, how many
    frames changed, how many were missed, and the area they changed in.
  * `record <id> frames` lists the changed frames with their time and
    changed area, as text.
  * `record <id> export` writes the chosen frames as images, one image per
    frame, and prints one path per line, so
    `... export --frames f005-f010 | xargs demi file view` shows them.
    `--frames` takes ranges and single frames separated by commas
    (`f005-f010,f020`). The frames of one export share one crop, the
    union of the areas they changed in, so they compare; `--whole` and
    `--region` choose otherwise. Each image is labelled with its frame
    number and time outside the picture. `--output` chooses the directory;
    without it the images go to the conversation's scratch directory.
  * `record <id> export --mp4` gives the recording's video itself.
* **Not tiled.** Frames are never put together into sheets: a blind test
  with weak models found no measurable difference between one frame per
  image and two or four per sheet, tiles shrunk to fit nine or more per
  sheet were read worse, and one image per frame needs no layout.
* **Limits.** A recording stops itself at two minutes or 200 MB, whichever
  comes first, and when its tab or device closes, and says so; a duration
  longer than two minutes is refused. So a recording an agent forgets to
  stop ends on its own. A conversation keeps its five newest
  recordings on the Host; a sixth removes the oldest, and archiving or
  deleting the conversation removes them all.
* The panel's Record button records the same way and saves the video. The
  systems' own recorders are no longer used. `bun browse` does not record;
  it is to be replaced by `demi browser`.

### Consequences

* Good, because a frame that shows a defect for 16 ms is kept and found, and
  a model reads only the frames that differ, at the size of what changed.
* Good, because the stream and the recording cannot disagree about what the
  screen showed, and the stream no longer decides how often the screen is
  captured.
* Bad, because an exported frame is decoded from a compressed video, so its
  fine detail is not the screen's exact pixels.
* Bad, because capturing every drawn frame costs more on the Host while a
  view is looked at than capturing at the stream's rate.
* Bad, because no source promises every frame. On probes at 60 Hz, each of
  the three ways above took every frame of three 20-second runs, the
  emulator's at its own pixels through shared memory included, where a
  software encoder of those pixels could not keep up; a busy Host made the
  Simulator skip one frame in a run. A recording says how many it
  missed rather than claiming none.
* Bad, because each Host system needs its own hardware encoder, and a
  Linux Host records Android at the device's logical size, not its pixels.
* Bad, because an export decodes the video with Cisco's OpenH264, which a
  Host downloads the first time it exports, a Mac that records only iOS
  included.
