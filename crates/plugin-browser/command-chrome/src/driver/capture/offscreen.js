// Captures the tabs the live view watches and the tabs being recorded, and
// encodes them as H.264.
//
// The live view module on this Host drives it over a loopback socket that
// accepts only this environment's token, and names the live protocol's codec.
//
// Chrome grants one capture of a tab, so a tab has one source that reads its
// frames, and each frame goes to every consumer of the tab:
//
//   tab capture --frames--> source --+--> live view: the newest, scaled to the
//   (the tab's own pixels)           |    stream's size, at the pace the
//                                    |    viewers' windows allow
//                                    +--> recording: every one that differs
//                                         from the one before, exactly, coded
//                                         at a fixed quality
//
// A recording never waits for a viewer, and a viewer never waits for a
// recording.
import { socket as address, codec } from './config.js';

// A frame's header, big-endian: capture (u32), sequence (u32), flags (u8,
// 1 = key frame, 2 = the area that changed follows), three reserved bytes,
// timestamp in microseconds (f64), width and height in pixels (u16 each), and
// the area that changed, x, y, width and height (u16 each), which a
// recording's pictures have.
const HEADER = 32;
const socket = new WebSocket(address);
socket.binaryType = 'arraybuffer';
// The consumers by capture ID, and the sources by CDP target.
const captures = new Map();
const sources = new Map();
let tasks = Promise.resolve();

// The frame rate a recording's encoder is told, which fixes nothing it keeps:
// a recording takes every frame the tab draws, coded at a fixed quality.
const RECORDING_FPS = 60;
// The most frames a second a tab's capture may deliver, which every capture of
// a tab is opened at, so that the source never depends on which consumer came
// first. A cap at 60 skips every second frame of a 120 Hz screen without a
// word. 120 is the most Chrome takes: with a larger one it falls back to 30
// frames a second, measured at 144 and 240.
const SOURCE_FPS = 120;
// A recording's quantizer, in H.264's 0 to 51, where lower keeps more. At 18
// a one-pixel move of text shows.
const QUANTIZER = 18;
// Frames waiting in a recording, to be compared or coded, beyond which a new
// frame is not taken and counts as missed, so a slow Host costs frames,
// never a broken video.
const MAX_ENCODER_QUEUE = 8;
// The most bytes the socket may hold unsent before a recording fails: its
// pictures cannot be dropped.
const MAX_BUFFERED = 64 * 1024 * 1024;
// Frames the source's reader may hold unread before the browser drops the
// oldest, which a recording counts as missed.
const SOURCE_BUFFER = 32;

function send(message) {
  if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify(message));
}

function fail(capture, error) {
  send({ type: 'error', capture, message: String(error?.message ?? error) });
}

function ask(message) {
  return chrome.runtime.sendMessage(message).then(answer => {
    if (answer?.error) throw new Error(answer.error);
    return answer;
  });
}

// H.264 4:2:0 encodes only even sides. Chrome already scales a tab with an
// odd side to an even size of its shape; a picture that still has an odd
// side is encoded without its last row or column.
function even(length) {
  return Math.max(2, length - (length % 2));
}

function pack(chunk, state) {
  // The sides the picture was encoded at, which a newer picture may have changed.
  const sides = state.sides.get(chunk.timestamp) ?? state;
  state.sides.delete(chunk.timestamp);
  const data = new ArrayBuffer(HEADER + chunk.byteLength);
  const view = new DataView(data);
  view.setUint32(0, state.id);
  view.setUint32(4, ++state.sequence);
  const changed = state.changes?.get(chunk.timestamp);
  state.changes?.delete(chunk.timestamp);
  view.setUint8(8, (chunk.type === 'key' ? 1 : 0) | (changed ? 2 : 0));
  view.setFloat64(12, chunk.timestamp);
  view.setUint16(20, sides.width);
  view.setUint16(22, sides.height);
  if (changed) {
    view.setUint16(24, changed.x);
    view.setUint16(26, changed.y);
    view.setUint16(28, changed.width);
    view.setUint16(30, changed.height);
  }
  chunk.copyTo(new Uint8Array(data, HEADER));
  return data;
}

// The pinned Chrome's Linux software capture produces BT.601 I420 samples
// labeled BT.709. Keep the samples and correct only their label, as verified
// against the page's own pixels; every Chrome upgrade checks it again.
async function corrected(frame) {
  if (frame.format !== 'I420' || frame.colorSpace.matrix !== 'bt709' || frame.colorSpace.fullRange !== false) {
    return frame;
  }
  try {
    const rect = { x: 0, y: 0, width: frame.codedWidth, height: frame.codedHeight };
    const pixels = new Uint8Array(frame.allocationSize({ rect }));
    const layout = await frame.copyTo(pixels, { rect });
    return new VideoFrame(pixels, {
      format: frame.format,
      codedWidth: frame.codedWidth,
      codedHeight: frame.codedHeight,
      visibleRect: frame.visibleRect,
      displayWidth: frame.displayWidth,
      displayHeight: frame.displayHeight,
      timestamp: frame.timestamp,
      ...(frame.duration === null ? {} : { duration: frame.duration }),
      colorSpace: { ...frame.colorSpace.toJSON(), matrix: 'smpte170m' },
      layout,
      transfer: [pixels.buffer],
    });
  } finally {
    frame.close();
  }
}

// Opens the capture of the tab of CDP target `target` at exactly these pixels,
// the tab's own: a recording keeps them whatever a viewer is sent.
async function openSource(target, width, height) {
  const { tabId, streamId } = await ask({ type: 'grant', target });
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: false,
    video: {
      mandatory: {
        chromeMediaSource: 'tab',
        chromeMediaSourceId: streamId,
        minWidth: width,
        maxWidth: width,
        minHeight: height,
        maxHeight: height,
        maxFrameRate: SOURCE_FPS,
      },
    },
  }).catch(error => {
    // Chrome can retain a pending tabCapture request after getUserMedia aborts.
    // There is no track or API to cancel that grant; the Host recreates this
    // extension, which clears Chrome's registry without replacing the user's
    // tabs or documents.
    if (error.name === 'AbortError') send({ type: 'recreate' });
    throw error;
  });
  const track = stream.getVideoTracks()[0];
  track.contentHint = 'detail';
  const source = {
    target,
    tabId,
    stream,
    track,
    // A buffer that rides out a pause of this document's thread, such as a
    // frame being compared, where the default keeps only the newest frame.
    reader: new MediaStreamTrackProcessor({ track, maxBufferSize: SOURCE_BUFFER }).readable.getReader(),
    width,
    height,
    consumers: new Set(),
    // The newest frame, which a consumer that joins gets at once: a still
    // tab draws nothing new for it.
    last: null,
    // When `last` arrived, on this document's clock.
    lastAt: 0,
    captured: 0,
  };
  sources.set(target, source);
  void pump(source);
  return source;
}

// Reads the source's frames and gives each to every consumer, until the
// source closes.
async function pump(source) {
  try {
    while (sources.get(source.target) === source) {
      const { value, done } = await source.reader.read();
      if (done) break;
      const frame = await corrected(value);
      if (sources.get(source.target) !== source) {
        frame.close();
        break;
      }
      source.captured++;
      source.last?.close();
      source.last = frame.clone();
      source.lastAt = performance.now();
      for (const consumer of [...source.consumers]) deliver(consumer, frame);
      frame.close();
    }
  } catch (error) {
    if (sources.get(source.target) === source) {
      for (const consumer of [...source.consumers]) {
        fail(consumer.id, error);
        void stop(consumer.id);
      }
    }
  }
}

// A consumer failing does not end the others.
function deliver(consumer, frame) {
  try {
    consumer.frame(frame);
  } catch (error) {
    fail(consumer.id, error);
    void stop(consumer.id);
  }
}

// Sources being released, until Chrome released their tabs.
const releasing = new Map();
// Captures being stopped, until Chrome released their tabs. A failure stops
// its capture at once; the module's stop of it then waits for the same
// release, since its `stopped` tells the module the tab may be resized.
const stopping = new Map();

// Stops the capture and releases the tab, which Chrome does after a moment.
function closeSource(source) {
  if (sources.get(source.target) === source) sources.delete(source.target);
  source.last?.close();
  source.last = null;
  for (const track of source.stream.getTracks()) track.stop();
  const released = (async () => {
    await source.reader.cancel().catch(() => {});
    await ask({ type: 'released', tabId: source.tabId }).catch(error => console.warn(error));
  })().finally(() => {
    if (releasing.get(source.target) === released) releasing.delete(source.target);
  });
  releasing.set(source.target, released);
  return released;
}

// The tab's source for a consumer: the one that runs, or a new one at
// `width` × `height`, the tab's own pixels. A tab's size changes only in a
// resize, which waits until Chrome released every capture of the tab, so a
// source is never at a size the tab no longer has.
async function acquire(target, width, height) {
  await releasing.get(target);
  return sources.get(target) ?? openSource(target, width, height);
}

// Detaches the capture's consumer, and closes its tab's source with the last
// one. Answers once Chrome released the tab, if it had to.
function stop(id) {
  const consumer = captures.get(id);
  if (!consumer) return stopping.get(id) ?? Promise.resolve();
  captures.delete(id);
  const closing = consumer.close();
  const source = consumer.source;
  source.consumers.delete(consumer);
  const released = (async () => {
    await closing;
    if (source.consumers.size === 0 && sources.get(source.target) === source) await closeSource(source);
    else await releasing.get(source.target);
  })().finally(() => stopping.delete(id));
  stopping.set(id, released);
  return released;
}

// Reconfiguring the same sides keeps the stream: its next frame is a delta
// frame, so a new budget or a still picture's refinement costs no key frame.
//
// The hardware encoder where the browser has one, as the recording's: a tab
// scrolled for 20 seconds took 65% of a core to watch with the software encoder
// and 41% with the hardware's, and the viewers' pictures were the same.
//
// No content hint: with `text` Chrome's software H.264 encoder, where the
// browser has no hardware one, runs its screen-content mode, whose rate control
// ignores the budget. Measured on scrolling text at 1072 x 1824, it kept every
// delta frame near 2 KiB and 39.7 dB PSNR at 20 and at 70 Mbps alike, while the
// default mode spends the budget: 47 dB at 70 Mbps, 38 dB at 10.
function configure(state, still) {
  state.encoder.configure({
    codec,
    width: state.width,
    height: state.height,
    framerate: still ? 1 : state.fps,
    bitrate: state.bitrate,
    bitrateMode: 'variable',
    latencyMode: 'realtime',
    hardwareAcceleration: 'prefer-hardware',
    avc: { format: 'annexb' },
  });
  state.still = still;
}

// Encode the latest picture when any of these holds:
//
// - it changed;
// - a key frame is due;
// - it has stood still long enough to send once more at higher quality.
//
// Encode only while the viewers' windows have room for it: the module says how
// many frames past the newest they had may come, and a picture that comes
// meanwhile replaces the one waiting. A frame in the encoder counts as sent.
function encode(state) {
  if (!state.latest || state.sequence + state.encoding - state.ack >= state.window) return;
  if (socket.bufferedAmount > 1024 * 1024) return;
  if (performance.now() < state.nextEncode) return;
  const refine = !state.settled && performance.now() - state.lastCaptureAt >= 500;
  if (!state.dirty && !state.force && !refine) return;
  if (!state.encoder) {
    state.encoder = new VideoEncoder({
      output(chunk) {
        if (captures.get(state.id) !== state) return;
        state.encoding--;
        socket.send(pack(chunk, state));
      },
      error(error) {
        fail(state.id, error);
        void stop(state.id);
      },
    });
  }
  // The picture is sent at the size the viewers' stream has now, scaled by
  // the encoder from the tab's own pixels.
  const width = even(state.sendWidth);
  const height = even(state.sendHeight);
  // The first picture, or the first at a new size, starts the encoding at its
  // sides with a key frame. A still picture is encoded once more with a
  // second's budget, and the next change goes back to the frame rate's.
  if (state.width !== width || state.height !== height) {
    state.width = width;
    state.height = height;
    configure(state, refine);
    state.force = true;
  } else if (state.still !== refine) {
    configure(state, refine);
  }
  const visible = state.latest.visibleRect;
  const timestamp = Math.round(performance.now() * 1000);
  state.sides.set(timestamp, { width, height });
  const frame = new VideoFrame(state.latest, {
    timestamp,
    visibleRect: {
      x: visible.x,
      y: visible.y,
      width: even(state.latest.displayWidth),
      height: even(state.latest.displayHeight),
    },
    displayWidth: width,
    displayHeight: height,
  });
  try {
    state.encoder.encode(frame, { keyFrame: state.force });
    state.encoding++;
  } finally {
    frame.close();
  }
  state.settled = refine;
  state.force = false;
  state.dirty = false;
  state.nextEncode = Math.max(state.nextEncode + 1000 / state.fps, performance.now());
}

// The live view's consumer of a tab: encodes the newest frame when the
// viewers' windows have room for it.
function liveConsumer(message, source) {
  const state = {
    kind: 'live',
    source,
    id: message.capture,
    fps: message.fps,
    bitrate: message.bitrate,
    window: 1,
    sequence: 0,
    // Frames in the encoder, which the window counts as sent.
    encoding: 0,
    ack: 0,
    force: true,
    dirty: false,
    settled: false,
    still: false,
    latest: null,
    encoder: null,
    nextEncode: 0,
    lastCaptureAt: 0,
    captured: 0,
    width: 0,
    height: 0,
    // The size of the pictures sent: the stream's, which the encoder scales
    // the tab's own pixels to.
    sendWidth: message.width,
    sendHeight: message.height,
    // Each picture in the encoder's queue with the sides it is encoded at.
    sides: new Map(),
    frame(frame) {
      state.latest?.close();
      state.latest = frame.clone();
      state.lastCaptureAt = performance.now();
      state.settled = false;
      state.dirty = true;
      state.captured++;
    },
    begin() {
      state.timer = setInterval(() => {
        try {
          encode(state);
        } catch (error) {
          fail(state.id, error);
          void stop(state.id);
        }
      }, 8);
      watch(state, message);
      if (source.last) state.frame(source.last);
    },
    close() {
      clearInterval(state.timer);
      clearTimeout(state.watchdog);
      state.latest?.close();
      if (state.encoder && state.encoder.state !== 'closed') state.encoder.close();
    },
  };
  return state;
}

// A page that stopped painting before capture began delivers no frame: the
// module forces a repaint, and owns retry after a second silence.
function watch(state, message) {
  state.watchdog = setTimeout(() => {
    if (captures.get(state.id) !== state || state.captured) return;
    send({ type: 'stalled', capture: state.id });
    state.watchdog = setTimeout(() => {
      if (captures.get(state.id) !== state || state.captured) return;
      tasks = tasks.then(async () => {
        // A stop queued before this failure may already have retired it.
        if (captures.get(state.id) !== state) return;
        fail(state.id, 'tab capture produced no frames');
        await stop(state.id);
      }).catch(error => fail(message.capture, error));
    }, 2000);
  }, 1500);
}

// Whether the browser codes at a fixed quantizer, which keeps the quality of
// a recording whatever the picture holds; not every encoder does.
let quantizing = null;
function quantizer(width, height) {
  quantizing ??= VideoEncoder.isConfigSupported({ ...encoding(width, height), bitrateMode: 'quantizer' })
    .then(result => result.supported === true, () => false);
  return quantizing;
}

// A recording's encoder settings: the hardware's encoder where there is one,
// and nothing that trades quality for a pace.
function encoding(width, height) {
  return {
    codec,
    width,
    height,
    framerate: RECORDING_FPS,
    latencyMode: 'quality',
    hardwareAcceleration: 'no-preference',
    avc: { format: 'annexb' },
  };
}

// The planes of a frame as the pixels they hold, for comparing two frames and
// saying where they differ: for each plane its bytes per row and how many of
// the frame's pixels a sample and a row of it cover, across and down.
function planes(format, width, height) {
  switch (format) {
    case 'I420':
      return [
        { bytes: width, across: 1, down: 1, step: 1 },
        { bytes: width / 2, across: 2, down: 2, step: 1 },
        { bytes: width / 2, across: 2, down: 2, step: 1 },
      ].map(plane => ({ ...plane, rows: height / plane.down }));
    case 'NV12':
      return [
        { bytes: width, across: 1, down: 1, step: 1, rows: height },
        { bytes: width, across: 2, down: 2, step: 2, rows: height / 2 },
      ];
    case 'RGBA':
    case 'RGBX':
    case 'BGRA':
    case 'BGRX':
      return [{ bytes: width * 4, across: 1, down: 1, step: 4, rows: height }];
    default:
      return null;
  }
}

// Where `now` differs from `before`, exactly, in the frame's pixels, or null
// when they are the same picture. Both are the bytes `copyTo` wrote, with the
// `layout` it gave.
function differs(now, before, layout, geometry) {
  let top = Infinity;
  let bottom = -1;
  let left = Infinity;
  let right = -1;
  geometry.forEach((plane, index) => {
    const { offset, stride } = layout[index];
    for (let row = 0; row < plane.rows; row++) {
      const start = offset + row * stride;
      const end = start + plane.bytes;
      let first = start;
      // Words first, as most rows are the same.
      while (first + 4 <= end && now[first] === before[first] && now[first + 1] === before[first + 1]
          && now[first + 2] === before[first + 2] && now[first + 3] === before[first + 3]) first += 4;
      while (first < end && now[first] === before[first]) first++;
      if (first === end) continue;
      let last = end - 1;
      while (now[last] === before[last]) last--;
      const y = row * plane.down;
      top = Math.min(top, y);
      bottom = Math.max(bottom, y + plane.down - 1);
      left = Math.min(left, Math.floor((first - start) / plane.step) * plane.across);
      right = Math.max(right, (Math.floor((last - start) / plane.step) + 1) * plane.across - 1);
    }
  });
  if (bottom < 0) return null;
  return { x: left, y: top, width: right - left + 1, height: bottom - top + 1 };
}

// The recording's consumer of a tab: keeps every frame that differs from the
// one before, exactly, codes it, and hands the pictures to the Host with where
// they differ, as they come. Frames the Host asks to be kept are never dropped
// after coding, since the next picture needs the one before. A frame the tab
// drew again unchanged is not coded: the same pixels cost nothing.
async function recordingConsumer(message, source) {
  const quantized = await quantizer(source.width, source.height);
  const state = {
    kind: 'record',
    source,
    id: message.capture,
    fps: RECORDING_FPS,
    // A key frame every this many frames, which the Host says: every source
    // of a recording has the same.
    keyInterval: message.keyInterval,
    sequence: 0,
    sides: new Map(),
    // Where each picture in the encoder differs from the one before it.
    changes: new Map(),
    encoder: null,
    width: 0,
    height: 0,
    sinceKey: 0,
    missed: 0,
    // The frames the track had discarded when the recording began.
    discarded: source.track.stats?.discardedFrames ?? 0,
    captured: 0,
    // The frames being compared and coded, in order.
    pending: 0,
    work: Promise.resolve(),
    // The frame kept last, as the bytes `copyTo` wrote, to compare the next
    // with, and the buffer the next is written to.
    before: null,
    now: null,
    frame(frame) {
      state.captured++;
      if (state.pending >= MAX_ENCODER_QUEUE) {
        state.missed++;
        return;
      }
      state.pending++;
      const held = frame.clone();
      state.work = state.work.then(() => state.keep(held)).catch(error => {
        fail(state.id, error);
        void stop(state.id);
      }).finally(() => {
        held.close();
        state.pending--;
      });
    },
    async keep(frame) {
      if (state.retired) return;
      const width = even(frame.displayWidth);
      const height = even(frame.displayHeight);
      const visible = frame.visibleRect;
      const rect = { x: visible.x, y: visible.y, width, height };
      const geometry = planes(frame.format, width, height);
      if (!geometry) throw new Error(`a captured frame has the format ${frame.format}, which cannot be compared`);
      const size = frame.allocationSize({ rect });
      const resized = state.width !== width || state.height !== height || state.now?.length !== size;
      if (resized) {
        state.before = null;
        state.now = new Uint8Array(size);
      }
      const layout = await frame.copyTo(state.now, { rect });
      // Compared with the frame kept last, not the frame before it: a slow
      // change adds up until it shows.
      const changed = state.before ? differs(state.now, state.before, layout, geometry) : { x: 0, y: 0, width, height };
      if (!changed) return;
      if (!state.encoder) {
        state.encoder = new VideoEncoder({
          output(chunk) {
            // A recording that was stopped still delivers what its encoder
            // flushes: those pictures were drawn before the stop.
            if (state.retired) return;
            if (socket.bufferedAmount > MAX_BUFFERED) {
              fail(state.id, 'the Host does not take the recording as fast as it is made');
              void stop(state.id);
              return;
            }
            socket.send(pack(chunk, state));
          },
          error(error) {
            fail(state.id, error);
            void stop(state.id);
          },
        });
      }
      let key = ++state.sinceKey >= state.keyInterval;
      if (state.width !== width || state.height !== height) {
        state.width = width;
        state.height = height;
        state.encoder.configure(quantized
          ? { ...encoding(width, height), bitrateMode: 'quantizer' }
          : { ...encoding(width, height), bitrateMode: 'variable', bitrate: Math.min(200_000_000, width * height * RECORDING_FPS) });
        key = true;
      }
      if (key) state.sinceKey = 0;
      state.sides.set(frame.timestamp, { width, height });
      state.changes.set(frame.timestamp, changed);
      const coded = new VideoFrame(frame, {
        timestamp: frame.timestamp,
        visibleRect: rect,
        displayWidth: width,
        displayHeight: height,
      });
      try {
        state.encoder.encode(coded, quantized ? { keyFrame: key, avc: { quantizer: QUANTIZER } } : { keyFrame: key });
      } finally {
        coded.close();
      }
      // The bytes just compared are the frame kept: the next is compared with
      // them, and the other buffer is written next.
      [state.before, state.now] = [state.now, state.before ?? new Uint8Array(size)];
    },
    begin() {
      // What the encoder dropped is reported in batches.
      state.reporting = setInterval(() => report(state), 250);
      watch(state, message);
      // The tab as it is now is the recording's first frame, stamped now: a
      // still tab draws nothing new for it.
      if (source.last) {
        const now = source.last.timestamp + Math.round((performance.now() - source.lastAt) * 1000);
        const current = new VideoFrame(source.last, { timestamp: now });
        try {
          state.frame(current);
        } finally {
          current.close();
        }
      }
    },
    async close() {
      clearInterval(state.reporting);
      clearTimeout(state.watchdog);
      // The frames still being compared were drawn before the stop.
      await state.work;
      report(state);
      if (state.encoder && state.encoder.state === 'configured') {
        // The pictures still in the encoder were drawn before the stop, and
        // are delivered as the encoder gives them.
        await state.encoder.flush().catch(() => {});
      }
      state.retired = true;
      if (state.encoder && state.encoder.state !== 'closed') state.encoder.close();
    },
  };
  return state;
}

// Tells the Host how many frames the recording lost since it last did: those
// it did not take, and those the browser discarded before they reached it,
// which the track counts (a source that does not count says none).
function report(state) {
  const discarded = state.source.track.stats?.discardedFrames;
  if (discarded !== undefined) {
    state.missed += Math.max(0, discarded - state.discarded);
    state.discarded = discarded;
  }
  if (state.missed === 0) return;
  send({ type: 'missed', capture: state.id, count: state.missed });
  state.missed = 0;
}

async function start(message) {
  if (socket.readyState !== WebSocket.OPEN) return;
  await stop(message.capture);
  const live = message.type === 'start';
  const source = await acquire(message.target, live ? message.sourceWidth : message.width, live ? message.sourceHeight : message.height);
  const consumer = live ? liveConsumer(message, source) : await recordingConsumer(message, source);
  captures.set(consumer.id, consumer);
  source.consumers.add(consumer);
  // The socket may have closed while Chrome was opening this media source.
  if (socket.readyState !== WebSocket.OPEN) {
    await stop(consumer.id);
    return;
  }
  consumer.begin();
  send({ type: 'started', capture: consumer.id });
}

socket.addEventListener('message', event => {
  const message = JSON.parse(event.data);
  const state = captures.get(message.capture);
  switch (message.type) {
    case 'start':
    case 'record':
    case 'stop':
      tasks = tasks
        .then(() => (message.type === 'stop' ? stop(message.capture) : start(message)))
        .then(() => message.type === 'stop' && send({ type: 'stopped', capture: message.capture }))
        .catch(error => fail(message.capture, error));
      break;
    case 'ack':
      if (!state) break;
      state.ack = Math.max(state.ack, Math.min(state.sequence, message.sequence));
      state.window = message.window;
      break;
    case 'keyframe':
      if (state) state.force = true;
      break;
    case 'encoding':
      if (!state) break;
      state.bitrate = message.bitrate;
      state.fps = message.fps;
      if (state.encoder) configure(state, state.still);
      state.settled = false;
      break;
  }
});
socket.addEventListener('open', () => send({ type: 'ready' }));
socket.addEventListener('close', () => {
  for (const id of [...captures.keys()]) void stop(id);
});
