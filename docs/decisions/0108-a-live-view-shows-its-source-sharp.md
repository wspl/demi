---
status: proposed
---

# A live view shows its source sharp

## Problem

Alex watches an iOS simulator in the panel on the Mac it runs on. The
simulator opens on a list of settings. The first picture is a key frame coded
for 5 Mbps, and the text in it is soft, with gray blotches on the light
background. The screen then stands still, so the simulator draws nothing new
and the Host sends nothing: the soft picture stays until Alex touches the
screen. When Alex scrolls, every picture again gets the budget of 5 Mbps,
although the page and the Host are on the same Mac and the path could carry
a hundred times that. And the text is a little softer than in Simulator even
when the picture is still.

Each of these has its own cause:

1. **A still picture is never refined.** An iOS or Android source sends a
   frame only when its screen changes. The last frame of a change keeps the
   quality its rate control gave it. The browser's capture refines a still tab
   once, with one more frame coded at a second's budget, and only after 500 ms
   without a change, which a blinking caret never leaves it.
2. **The bit rate stays at its start.** The rate control (`live-view`'s
   `rate`) starts at 5 Mbps and raises its estimate only to 1.5 times the
   rate it observed, as Google Congestion Control's increase does. A picture
   of text rarely spends its whole budget, so the estimate never passes 5
   Mbps, even on a loopback path. Nothing probes the path.
3. **The simulators' encoders lack VideoToolbox's low-latency rate
   control.** The iOS helper and the Android capture on macOS open
   VideoToolbox without it. Chrome's WebCodecs encoder, which codes the
   browser's tab, opens it with it in real-time mode.
4. **The iOS pictures are not the panel's size.** A device's pictures take
   the largest size any viewer has shown it at since its capture began. A
   panel that shrinks shows them scaled by a fraction, such as 812 pixels
   into 781, and the browser's scaling softens every edge.

## Decision

A live view's picture is as sharp as the path allows while it moves, and
reaches the encoder's best within a second of standing still, on every
source, at the panel's own pixels.

### A still picture is coded again for a second

The stream (`live-view`'s `stream`) owns when a picture is refined, for every
source. When a capture has sent no new frame for one frame interval, the
stream asks it to code its last picture once more, as a delta frame, at each
frame interval, for one second. Each repeat is coded against the one before,
so the encoder spends its budget on the parts that are still coarse, until
there are none and a repeat costs a hundred bytes or so. A new picture from
the source stops the repeats, and its own stillness starts them again. The
source marks the frame that answers a repeat, so a picture that changes while
a repeat is asked for is not taken for its answer and gets a whole second of
its own.

```text
picture   K
repeats        r    r    r    r    r    r  ...  r    r     (one second)
bytes     178k 23k  0.5k 1.0k 4.9k 14k  20k ... 0.2k 0.1k
PSNR      37 dB                         40  ... 42   42 dB
```

```rust
pub trait Capture: Send + 'static {
    // ...
    /// Codes the picture the source shows now once more, as a delta frame
    /// marked as a repeat; a picture that changed meanwhile is coded instead,
    /// unmarked.
    fn repeat(&self);
}
```

- The browser's capture extension codes its latest frame again. Its own
  one-shot refinement, a second's budget after 500 ms of stillness, goes.
- The iOS helper codes the screen's current surface again, through a new
  `repeat` request.
- The Android capture codes its last scaled picture again.
- The browser's and the iOS helper's frame headers gain a flag for a repeat,
  in their reserved flag bits; `EncodedFrame` gains `repeat`.
- A repeat counts against the viewers' window like any frame, so a slow path
  holds the repeats back as it holds pictures.

### The rate control probes the path

The rate control sends probes as Google Congestion Control's probe controller
does for a screen share:

- With a view's first picture, two probes, at 3 and 6 times the estimate.
- While the last probe finds the path carries at least 70% of its rate,
  another at twice what it found.
- While the pictures spend less than 65% of the estimate, one probe every 5
  seconds at twice the estimate, after a picture, which probes further as the
  first ones do. A still screen sends no pictures, so it sends no probes
  either.
- A probe is a burst of padding chunks at its rate, 15 ms long and at least
  5 chunks, at most the codec level's 240 Mbps.
- The page acknowledges each padding chunk with its receive time, as it does
  a picture's, and drops it.
- What a probe measured, the least of its send rate and its receive rate
  when at least 80% of its chunks came, becomes the estimate, as libwebrtc's
  delay-based estimate takes it. The AIMD rate control already takes a
  probe's result; it gains the probe estimator, ported from libwebrtc
  through str0m as its delay-based half was, and the rules above, which are
  `live-view`'s own.
- A probe measures once, when enough of its chunks came; a chunk after that
  does not count. libwebrtc measures again at every packet, since a lost
  padding packet never arrives; the video channel here sends a lost chunk
  again, and a chunk that arrives hundreds of milliseconds late would make a
  30 Mbps path measure under 1 Mbps.

On a path that carries 100 Mbps, the encoder is given 100 Mbps within the
first second, and keeps it while it spends less, until the delay grows.

The live view's protocol gains the padding chunk:

```rust
/// A video chunk's flags (`VideoHeader`): 1 a key frame's, 2 padding, which
/// the page acknowledges and drops.
pub const PADDING_FLAG: u8 = 2;
```

A padding chunk carries the stream's generation and a sequence of its own,
and its acknowledgement carries the flag in one of its reserved bytes, so a
padding chunk and a picture's chunk never share an ID. Page and Host ship in
the same release, so neither meets the other without it.

### The simulators' encoders run in low-latency mode

The iOS helper and the Android capture on macOS (`shared-h264`'s
VideoToolbox encoder at a bit rate) open VideoToolbox with
`kVTVideoEncoderSpecification_EnableLowLatencyRateControl`, as Claude
Desktop's simulator view and Chrome do. They set no frame delay count: the
hardware H.264 encoder refuses `kVTCompressionPropertyKey_MaxFrameDelayCount`
at any value, and without reordering it already hands back each picture before
it takes the next. An encoder that refuses the low-latency mode, such as the
software one, opens without it. A recording's encoder, which codes at a fixed
quality, keeps its settings.

### A device's pictures follow the panel's size

A device's pictures take the size the largest of its viewers shows them at
now, in device pixels, not the largest since the capture began, whether the
panel grew or shrank. The view already says its size only once it has stood
for 150 ms, so a drag of the panel's edge does not cost a key frame at every
step.

## Spike

**Does VideoToolbox improve a picture coded again?** A probe coded a page of
text at 1074 × 1824 pixels (a Retina panel's) at 30 fps: one second still,
one second scrolling at 40 pixels a frame, then still. Each decoded frame was
compared with the same frame coded at 100 Mbps, in PSNR.

| Encoder settings     | Key frame | Repeats, 0.5 s | Scrolling |
| -------------------- | --------- | -------------- | --------- |
| 5 Mbps               | 37 dB     | 42 dB          | 43–45 dB  |
| 20 Mbps              | 42 dB     | 45 dB          | 47–49 dB  |
| 40 Mbps              | 45 dB     | 50 dB          | 52–53 dB  |
| 5 Mbps, low latency  | 39 dB     | 48 dB          | 47–48 dB  |

- Repeats raise the key frame from 37 to 42 dB within 15 frames, half a
  second, for 173 KB, and cost about 100 bytes each from the 20th on. With the
  low-latency rate control they reach 48 dB within 30 frames, after which
  each costs about 700 bytes. One second of repeats covers both.
- After scrolling, eight repeats raised the last picture from 44.9 to 45.2 dB
  for 19 KB. A forced key frame of the same picture at the same budget gave
  42.9 dB for 307 KB.
- `kVTCompressionPropertyKey_MaxAllowedFrameQP` changed nothing without the
  low-latency rate control, so the design does not use it.
- An iOS push transition at 812 × 1768 and 60 fps coded at 5 Mbps stayed
  above 44 dB, so a budget of 5 Mbps is not what blurs motion there; the key
  frame and the missing refinement are.

**Does the estimate rise on a loopback path?** In the development setup, a
browser tab watched at 1074 × 1824 and scrolled for 10 seconds spent 0.9 to
1.8 Mbps, and the estimate stayed at 5,000,000 bit/s with no change logged.

**What does the page show of an iOS device?** A canvas of 812 × 1768 pixels
drawn at 390.5 × 849 CSS pixels at a pixel ratio of 2, that is 781 × 1698
device pixels: the browser scales it by 0.962.

## Prior art

- **libwebrtc's screen share.** In zero-hertz mode
  (`video/frame_cadence_adapter.cc`), when the source stops sending frames,
  the encoder is fed the last frame again at the frame rate until the quality
  converges (`video/quality_convergence_monitor.cc`, by the frames' QP), then
  once a second. Demi repeats for one second and stops: it needs no frames to
  keep a receiver's freeze detection quiet, and a converged repeat costs a few
  hundred bytes, so a fixed second costs less than reading each slice's QP,
  which H.264 has no threshold for in libwebrtc either.
- **libwebrtc's probe controller**
  (`modules/congestion_controller/goog_cc/probe_controller.cc`): initial
  probes at 3 and 6 times the start rate, and periodic probes while the
  application limits the rate, every 5 seconds at twice the estimate, which
  libwebrtc runs for screen content under a field trial. str0m ports it with
  its probe estimator (`src/bwe/probe/`). Demi's probes are padding chunks on the video channel, where
  WebRTC's are RTP padding.
- **Claude Desktop 3.6556 (not open source; read from its bundle and run).**
  Its browser pane embeds a real browser view and streams no pictures. Its
  iOS simulator pane shares the simulator's surface with a native view where
  a flag allows, and otherwise codes H.264 with VideoToolbox in low-latency
  mode at the simulator's own pixels, 12 Mbps with bursts of 18 Mbps a
  second, a key frame every 4 seconds, BT.709. It neither adapts to the path
  nor refines a still picture; its periodic key frame refreshes it. It serves
  local sessions only, so a fixed rate fits it; Demi's Host may be far away.
- **Chrome's VideoToolbox encoder**
  (`media/gpu/mac/vt_video_encode_accelerator_mac.mm`) turns on the
  low-latency rate control when low delay is required and the hardware
  encoder supports it; it fails with the software encoder, so Demi opens
  without it where it fails.

## Alternatives

- **A key frame every few seconds, as Claude Desktop does.** After scrolling,
  a key frame of the still picture cost 16 times the bytes of the repeats and
  came out 2.3 dB worse, and it is sent whether or not the picture needs it.
- **Repeats until the QP stops falling, as libwebrtc does.** It needs a slice
  header parser for each encoder's output to save a few kilobytes a second.
- **A lossless or PNG still frame.** It would need a second decoder and a
  second drawing path on the page, and a PNG of a Retina screen is several
  megabytes, which a remote path cannot carry in half a second.
- **A fixed bit rate on a local path.** The Host cannot tell a local path
  from a fast one without measuring it, and a probe measures it.
- **The simulator's own pixels, scaled by the page.** It is what Claude
  Desktop's H.264 path does. It sends up to 2.4 times the pixels the panel
  shows and still scales by a fraction.

## Open questions

None.
