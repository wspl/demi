// Captures the tabs the live view watches and encodes them as H.264
// (`live-view.md` § Capture). The live view module on this Host
// drives it over a loopback socket that accepts only this environment's token,
// and names the live protocol's codec.
import { socket as address, codec } from './config.js';

// A frame's header, big-endian: capture (u32), sequence (u32), flags (u8,
// 1 = key frame), three reserved bytes, timestamp in microseconds (f64),
// width and height in pixels (u16 each), eight reserved bytes.
const HEADER = 32;
const socket = new WebSocket(address);
socket.binaryType = 'arraybuffer';
const captures = new Map();
let tasks = Promise.resolve();

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

// The tab's size as a capture constrains it: exactly these pixels, so the
// page is captured unscaled once its surface has this size.
function size(width, height) {
  return { width: { min: width, max: width }, height: { min: height, max: height } };
}

function pack(chunk, state) {
  // The sides the picture was encoded at, which a newer picture may have changed.
  const sides = state.sides.get(chunk.timestamp) ?? state;
  state.sides.delete(chunk.timestamp);
  const data = new ArrayBuffer(HEADER + chunk.byteLength);
  const view = new DataView(data);
  view.setUint32(0, state.id);
  view.setUint32(4, ++state.sequence);
  view.setUint8(8, chunk.type === 'key' ? 1 : 0);
  view.setFloat64(12, chunk.timestamp);
  view.setUint16(20, sides.width);
  view.setUint16(22, sides.height);
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

async function open(message) {
  const { tabId, streamId } = await ask({ type: 'grant', target: message.target });
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: false,
    video: {
      mandatory: {
        chromeMediaSource: 'tab',
        chromeMediaSourceId: streamId,
        minWidth: message.width,
        maxWidth: message.width,
        minHeight: message.height,
        maxHeight: message.height,
        maxFrameRate: message.fps,
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
  return { tabId, stream, track, reader: new MediaStreamTrackProcessor({ track }).readable.getReader() };
}

// Captures being stopped, until Chrome released them. A failure stops its
// capture at once; the module's stop of it then waits for the same release,
// since its `stopped` tells the module the tab may be resized.
const stopping = new Map();

function stop(id) {
  const state = captures.get(id);
  if (!state) return stopping.get(id) ?? Promise.resolve();
  captures.delete(id);
  clearInterval(state.timer);
  clearTimeout(state.watchdog);
  state.latest?.close();
  if (state.encoder && state.encoder.state !== 'closed') state.encoder.close();
  for (const track of state.stream.getTracks()) track.stop();
  const released = (async () => {
    await state.reader.cancel().catch(() => {});
    await ask({ type: 'released', tabId: state.tabId }).catch(error => console.warn(error));
  })().finally(() => stopping.delete(id));
  stopping.set(id, released);
  return released;
}

// Reconfiguring the same sides keeps the stream: its next frame is a delta
// frame, so a new budget or a still picture's refinement costs no key frame.
// No content hint: with `text` Chrome's software H.264 encoder runs its
// screen-content mode, whose rate control ignores the budget. Measured on
// scrolling text at 1072 x 1824, it kept every delta frame near 2 KiB and
// 39.7 dB PSNR at 20 and at 70 Mbps alike, while the default mode spends the
// budget: 47 dB at 70 Mbps, 38 dB at 10.
function configure(state, still) {
  state.encoder.configure({
    codec,
    width: state.width,
    height: state.height,
    framerate: still ? 1 : state.fps,
    bitrate: state.bitrate,
    bitrateMode: 'variable',
    latencyMode: 'realtime',
    hardwareAcceleration: 'prefer-software',
    avc: { format: 'annexb' },
  });
  state.still = still;
}

// Encode the latest picture when it changed, a key frame is due, or it has
// stood still long enough to send once more at higher quality, within the
// frames the module lets be in flight.
function encode(state) {
  if (!state.latest || state.sequence - state.ack >= state.window) return;
  if (socket.bufferedAmount > 1024 * 1024) return;
  if (state.encoder?.encodeQueueSize > 1 || performance.now() < state.nextEncode) return;
  const refine = !state.settled && performance.now() - state.lastCaptureAt >= 500;
  if (!state.dirty && !state.force && !refine) return;
  if (!state.encoder) {
    state.encoder = new VideoEncoder({
      output(chunk) {
        if (captures.get(state.id) !== state) return;
        socket.send(pack(chunk, state));
      },
      error(error) {
        fail(state.id, error);
        void stop(state.id);
      },
    });
  }
  const width = even(state.latest.displayWidth);
  const height = even(state.latest.displayHeight);
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
    visibleRect: { x: visible.x, y: visible.y, width, height },
    displayWidth: width,
    displayHeight: height,
  });
  try {
    state.encoder.encode(frame, { keyFrame: state.force });
  } finally {
    frame.close();
  }
  state.settled = refine;
  state.force = false;
  state.dirty = false;
  state.nextEncode = Math.max(state.nextEncode + 1000 / state.fps, performance.now());
}

async function start(message) {
  if (socket.readyState !== WebSocket.OPEN) return;
  await stop(message.capture);
  const source = await open(message);
  const state = {
    ...source,
    id: message.capture,
    fps: message.fps,
    bitrate: message.bitrate,
    window: 4,
    sequence: 0,
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
    // Each picture in the encoder's queue with the sides it is encoded at.
    sides: new Map(),
  };
  captures.set(state.id, state);
  // The socket may have closed while Chrome was opening this media source.
  if (socket.readyState !== WebSocket.OPEN) {
    await stop(state.id);
    return;
  }
  state.timer = setInterval(() => {
    try {
      encode(state);
    } catch (error) {
      fail(state.id, error);
      void stop(state.id);
    }
  }, 8);
  void (async () => {
    try {
      while (captures.get(state.id) === state) {
        const { value, done } = await state.reader.read();
        if (done) break;
        const frame = await corrected(value);
        if (captures.get(state.id) !== state) {
          frame.close();
          break;
        }
        state.latest?.close();
        state.latest = frame;
        state.lastCaptureAt = performance.now();
        state.settled = false;
        state.dirty = true;
        state.captured++;
      }
    } catch (error) {
      if (captures.get(state.id) === state) {
        fail(state.id, error);
        await stop(state.id);
      }
    }
  })();
  // A page that stopped painting before capture began delivers no frame:
  // the module forces a repaint, and owns retry after a second silence.
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
  send({ type: 'started', capture: state.id });
}

socket.addEventListener('message', event => {
  const message = JSON.parse(event.data);
  const state = captures.get(message.capture);
  switch (message.type) {
    case 'start':
    case 'stop':
      tasks = tasks
        .then(() => (message.type === 'start' ? start(message) : stop(message.capture)))
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
