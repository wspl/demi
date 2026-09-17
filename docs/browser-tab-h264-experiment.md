# Browser tab H.264 experiment

Experimental evidence recorded on 2026-09-18, not a selected product design or
Cloud acceptance. The authoritative product contract remains
[Conversation browser](demi-next/browser.md).

## Reading index

- [Capture and encoding](#capture-and-encoding): direct tab capture and decode.
- [Encoder comparison](#encoder-comparison): identical captured frames.
- [Adaptive quality](#adaptive-quality): tested changes and remaining work.

## Capture and encoding

Environment: Apple M5 Max, macOS arm64, Chrome for Testing 153.0.8010.36,
FFmpeg 8.1.2. Chrome ran headlessly with `--disable-gpu`; WebCodecs explicitly
requested `prefer-software` for encoding and decoding. This is not a benchmark
of the weaker Cloud CPU.

An isolated Manifest V3 extension with `tabCapture` and `tabs` permissions was
loaded into a temporary browser profile. Its ID was passed through
`--allowlisted-extension-id` to allow capture without a toolbar invocation.
An extension page obtained a stream ID for a specified tab, consumed it through
`getUserMedia`, and read frames through `MediaStreamTrackProcessor`.
No desktop/window capture, virtual display, JPEG stream, or VP8 intermediate
was used.

The fixture contained English and Chinese text, colored text, scrolling, and a
moving rectangle. It animated for 4.5 seconds, then stopped. The browser returned
1280×720 I420 frames. A WebCodecs encoder produced Annex B H.264 High Profile,
4:2:0, with no B-frames. Configuration: 30 fps, 1.5 Mbps, variable bitrate,
realtime latency. The same extension decoded the output with VideoDecoder.

The final run encoded and decoded 136 frames with no queue-limit drops. The
encode-call-to-output-callback latency was 1.4 ms median and 2.4 ms P95.
Output totaled 851,256 bytes. This latency excludes capture, network transfer,
and presentation. No frames arrived during the final 1.5-second static interval;
this observation is fixture-specific, not a guarantee for all static pages.

For comparison only, frames were also copied into packed I420 buffers and saved.
The experiment retained raw and compressed frames in memory and uploaded files
after capture. It is not a production streaming/backpressure implementation.

## Encoder comparison

All candidates encoded the same 136 saved frames, with timestamps normalized to
30 fps. x264 used `tune=zerolatency`, sliced threads, no scene-cut keyframes, and
one initial keyframe. WebCodecs used High Profile, software preference, variable
bitrate, realtime latency, and Annex B output.

| Encoder | Setting | Output bytes | Mean kbps at 30 fps | PSNR dB |
| --- | --- | ---: | ---: | ---: |
| WebCodecs | 0.5 Mbps | 282,629 | 499 | 34.38 |
| WebCodecs | 1.5 Mbps | 841,020 | 1,484 | 43.04 |
| WebCodecs | 3 Mbps | 1,292,892 | 2,282 | 47.14 |
| x264 superfast | CRF 20, 4 threads | 741,400 | 1,308 | 48.20 |
| x264 superfast | CRF 23, 4 threads | 550,102 | 971 | 45.32 |
| x264 superfast | CRF 28, 4 threads | 348,092 | 614 | 40.90 |
| x264 veryfast | CRF 23, 4 threads | 533,204 | 941 | 45.31 |
| x264 superfast | CRF 23, 1 thread | 526,256 | 929 | 45.44 |

PSNR compares decoded frames against captured I420, with both input timestamps
aligned by frame index. It does not measure text readability or loss introduced
before capture, including RGB-to-4:2:0 conversion. Increasing bitrate cannot
restore color detail already lost in that conversion.

On this fixture, x264 superfast CRF 20 used about 43% fewer bytes than WebCodecs
at a 3 Mbps target while achieving a higher PSNR. This is not a general claim
that x264 always wins: rate-control modes differ, content is limited, and there
is no network/VBV cap in the x264 runs.

The four-thread superfast CRF 23 run took 79 ms wall time and 271 ms total CPU
time for 136 frames. One thread took 238 ms wall time and 255 ms CPU time.
More threads reduced batch completion time but did not reduce total CPU cost.
Native results exclude live browser-to-native transfer; they do not establish
an end-to-end advantage. Raw 1280×720 I420 at 30 fps is about 41.5 MB/s per tab.

Local experiment artifacts are under `.cache/tab-h264-spike/`: `probe.mjs`,
`benchmark.mjs`, `compare.py`, raw/encoded samples, and JSON/log results.
These are development artifacts with machine-specific paths, not shipped tools.

## Adaptive quality

The same live encoder object was reconfigured through targets of
0.5 → 3 → 1 Mbps. Each segment contained 30 frames, began with a requested
keyframe, and was flushed before the next segment. The capture stream and
receiver decoder were not recreated. The decoder successfully output all
90 frames. Segment sizes were 83,424, 264,907, and 112,532 bytes respectively.

This proves reconfiguration and continuous decoder use in the local experiment.
It does not prove seamless wall-clock playback or automatic network adaptation:
frames were replayed in batches without a network bottleneck, and segment
boundaries included flushes and keyframes.

A future controller can use receiver progress and sender queue age to lower
bitrate promptly under congestion and probe upward gradually with hysteresis.
Resolution and frame rate require their own decisions; preserving text dimensions
is useful, but lowering frame rate also reduces scrolling smoothness. A static
page supplies too little traffic to establish available capacity. A retained
last frame can be encoded again at higher quality when capacity improves, even
when capture produces no new frames. This refinement remains unimplemented.

Still untested: constrained-network feedback, live browser-to-native transfer,
Linux Cloud performance, simultaneous tab captures, RGB/4:4:4 capture paths,
and comparative visual readability. No product, runner, or browser lifecycle
behavior changed in this experiment.

## H.264 4:4:4 follow-up

On the same machine and Chrome version, x264 encoded a synthetic RGB test chart
into a single-frame H.264 High 4:4:4 Predictive lossless sample (`yuv444p`, QP 0).
A separate High Profile 4:2:0 sample served as the control. The codec strings
were derived from each sample's SPS: `avc1.f40016` and `avc1.640016`.
This tests the client decoder independently; the 4:4:4 source was not a tab.

The WebCodecs test did not stop at `isConfigSupported()`: it configured a decoder,
submitted the Annex B keyframe, flushed, inspected the output VideoFrame format,
and drew it to an OffscreenCanvas with a pixel readback. Both default Chrome GPU
settings and `--disable-gpu` were tested.

| Path | Result on this Chrome/macOS installation |
| --- | --- |
| H.264 4:4:4 decode, software preference | Supported; decoded I444 and drew to canvas |
| H.264 4:4:4 decode, no preference | Supported; decoded I444 and drew to canvas |
| H.264 4:4:4 decode, hardware preference | Unsupported, including with default GPU settings |
| H.264 4:4:4 WebCodecs encoding | Unsupported for all three acceleration preferences |
| H.264 4:2:0 hardware decode, default GPU settings | Supported; decoded NV12 |
| tabCapture output, both GPU configurations | I420 |

The supported capture constraints exposed no pixel-format/chroma selector.
Consequently this experiment establishes a working Web client software decode
path for a static 4:4:4 frame, but not an end-to-end 4:4:4 tab stream. It does not
establish Chrome on other platforms, Safari, Firefox, or sustained decode speed.
Artifacts: `probe444.mjs`, `444-software.json`, `444-default.json`, and
`sample444.h264` in the same local experiment directory.

The pinned Chromium source has an internal RGB capture path:
[FrameSinkVideoCapturerImpl](https://raw.githubusercontent.com/chromium/chromium/153.0.8010.36/components/viz/service/frame_sinks/video_capture/frame_sink_video_capturer_impl.cc)
supports ARGB and maps it to RGBA copy output, including a shared-memory path.
This is an internal compositor interface, not evidence that an extension can
request RGB from tabCapture. Reaching that path would require an exposed API or
a native browser integration; neither was implemented here. Converting the
observed I420 frames to RGB or I444 after capture does not restore lost chroma.
