/**
 * The conversation's simulators without a Host (`mobile.md` § The live
 * view): the gallery's plugin, which gives each device the agent opens its
 * `mobile` tab and closes a device whose tab is closed, as the backend's
 * does, and a live view that draws a device's screen in its outline frame,
 * encodes it as the helper would and speaks the `mobile` protocol, so the
 * panel's tabs, controls, touches and the agent's activity show here.
 */
import { LIVE_CONTROL_FRAME, LIVE_VIDEO_CODEC, type LiveActivity, type ViewerInput } from '@demicodes/protocol'
import {
  closeResultSchema,
  onDeviceSchema,
  pressButtonSchema,
  recordDeviceSchema,
  rotateDeviceSchema,
  type LiveButtonMark,
  type LiveFrame,
  type LiveModuleMessage,
  type LiveViewerMessage,
  type MobilePlatform,
  type Orientation,
  mobileTabDataSchema,
} from '@demicodes/plugin-mobile'
import { PluginCallError } from '@demicodes/web-ui/plugins/page'
import { encodeControl, encodeVideo } from '@demicodes/web-ui/live/frames'
import type { OpenUserStream, UserStreamHandlers } from '@demicodes/web-ui/plugins/streams'
import type { PanelTab } from '@demicodes/web-ui/agent/panel-tabs'
import { productWould } from '../product-would'
import type { GalleryPanel } from './panel'
import type { GalleryPlugin } from './plugins'
import { WORKSPACE_ROOT } from './workspace'

const FPS = 10
/** A screen's pixels per point. */
const SCALE = 3
const decoder = new TextDecoder()

/** A simulator of the gallery's Mac: its screen upright in points, its corners and its buttons upright. */
interface GallerySimulator {
  name: string
  platform: MobilePlatform
  os: string
  nativeId: string
  points: { width: number; height: number }
  cornerRadius: number
  buttons: readonly LiveButtonMark[]
}

/** A device the gallery has open: its screen as the drawing shows it. */
interface OpenDevice {
  id: string
  simulator: GallerySimulator
  orientation: Orientation
  dark: boolean
  /** What the screen shows: the home screen or an app the viewer tapped. */
  screen: 'home' | 'settings'
  /** How far Settings is scrolled, in points, as the viewer's wheel scrolls it. */
  scrolled: number
  /** Whether the screen is off: the picture keeps what it last showed. */
  off: boolean
  /** The viewer's or the agent's last touch, drawn as a ring until it fades. */
  touch: { x: number; y: number; at: number } | null
  activity: LiveActivity
  recording: boolean
}

/** Settings' rows, in groups, longer than a screen so the wheel scrolls them. */
const SETTINGS_GROUPS: readonly (readonly string[])[] = [
  ['General', 'Accessibility', 'Display & Brightness', 'Privacy & Security', 'Developer'],
  ['Notifications', 'Sounds & Haptics', 'Focus', 'Screen Time'],
  ['Wallpaper', 'Siri', 'Camera', 'Battery', 'Storage'],
  ['Passwords', 'Wallet', 'Game Center', 'iCloud'],
]
const SETTINGS_ROW = 44
const SETTINGS_GAP = 24
/** Where Settings' first group starts, and its content's height, its last group's end with a margin below. */
const SETTINGS_TOP = 130
const SETTINGS_HEIGHT = SETTINGS_TOP + SETTINGS_GROUPS.reduce((height, group) => height + group.length * SETTINGS_ROW + SETTINGS_GAP, 0)

/** The simulators the agent opens, in turn: an iPhone, an iPad, then an Android emulator, as `demi mobile open` would. */
const SIMULATORS: readonly GallerySimulator[] = [
  {
    name: 'iPhone 17 Pro',
    platform: 'ios',
    os: 'iOS 26.5',
    nativeId: '5F1C2B3A-7D44-4E2B-9C11-0A1B2C3D4E5F',
    points: { width: 402, height: 874 },
    cornerRadius: 62,
    buttons: [
      { side: 'left', start: 0.18, length: 0.04 },
      { side: 'left', start: 0.24, length: 0.07 },
      { side: 'left', start: 0.33, length: 0.07 },
      { side: 'right', start: 0.29, length: 0.11 },
    ],
  },
  {
    name: 'iPad Air 13-inch (M3)',
    platform: 'ios',
    os: 'iOS 26.5',
    nativeId: '247C3132-2366-4D6C-A4B9-9051A0EE8EAC',
    points: { width: 1032, height: 1376 },
    cornerRadius: 18,
    buttons: [
      { side: 'top', start: 0.78, length: 0.08 },
      { side: 'right', start: 0.1, length: 0.05 },
      { side: 'right', start: 0.17, length: 0.05 },
    ],
  },
  {
    name: 'Pixel 8 API 36',
    platform: 'android',
    os: 'Android 16',
    nativeId: 'Pixel_8_API_36',
    points: { width: 411, height: 914 },
    cornerRadius: 24,
    buttons: [
      { side: 'right', start: 0.2, length: 0.06 },
      { side: 'right', start: 0.3, length: 0.13 },
    ],
  },
]

const LANDSCAPE: readonly Orientation[] = ['landscape-left', 'landscape-right']

/** The screen as the picture shows it, in points. */
function upright(device: OpenDevice): { width: number; height: number } {
  const { width, height } = device.simulator.points
  return LANDSCAPE.includes(device.orientation) ? { width: height, height: width } : { width, height }
}

const TURNS: readonly Orientation[] = ['portrait', 'landscape-left', 'portrait-upside-down', 'landscape-right']

type Side = LiveButtonMark['side']

/**
 * Where an upright edge is once the device is turned, and whether the edge
 * reads the other way along, as `demi-mobile` turns its marks.
 */
const TURNED: Record<Orientation, Record<Side, [Side, boolean]>> = {
  'portrait': { left: ['left', false], right: ['right', false], top: ['top', false], bottom: ['bottom', false] },
  'landscape-left': { left: ['bottom', false], right: ['top', false], top: ['left', true], bottom: ['right', true] },
  'landscape-right': { left: ['top', true], right: ['bottom', true], top: ['right', false], bottom: ['left', false] },
  'portrait-upside-down': { left: ['right', true], right: ['left', true], top: ['bottom', true], bottom: ['top', true] },
}

/** `mark` of the upright device on the edge it is on once the device is turned to `orientation`. */
function turned(mark: LiveButtonMark, orientation: Orientation): LiveButtonMark {
  const [side, reversed] = TURNED[orientation][mark.side]
  return { side, start: reversed ? Math.max(0, 1 - mark.start - mark.length) : mark.start, length: mark.length }
}

/** The device a `mobile` tab shows. */
function shows(tab: PanelTab): string | undefined {
  return mobileTabDataSchema.safeParse(tab.data).data?.device
}

function beat(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

export interface GalleryMobile {
  plugin: GalleryPlugin
  /** The panel tab of a device its user closed: the device closes, as `demi mobile close` does. */
  removed(tab: PanelTab): void
  /** The agent opens the next simulator with `demi mobile open`: its tab joins the panel. */
  agentOpens(): Promise<void>
  /** The agent taps each open device at a point of its screen, as `demi mobile tap` does. */
  agentTaps(): void
  /** The agent starts or stops recording the first open device, as `demi mobile record` does. */
  agentRecords(): void
  /** Each open device's screen turns off by itself, as a device's sleep turns it. */
  devicesSleep(): void
}

export function galleryMobile(panel: GalleryPanel): GalleryMobile {
  const open = new Map<string, OpenDevice>()
  let numbers = 0
  const views = new Set<MobileView>()

  function changed(): void {
    for (const view of views) {
      view.state()
    }
  }

  function device(id: string): OpenDevice {
    const found = open.get(id)
    if (!found) {
      throw new PluginCallError('device_not_found', `no device ${id} is open`, { type: 'status', status: '409 Conflict' })
    }
    return found
  }

  /** When each device's note goes: three seconds after the agent's last command on it. */
  const lingering = new Map<string, ReturnType<typeof setTimeout>>()

  /** The agent's command on `shown` ended now; its note stays three seconds more, as the product's does. */
  function agentUsed(shown: OpenDevice): void {
    shown.activity = { ...shown.activity, active: true }
    for (const view of views) {
      view.activity(shown)
    }
    clearTimeout(lingering.get(shown.id))
    lingering.set(shown.id, setTimeout(() => {
      lingering.delete(shown.id)
      shown.activity = { ...shown.activity, active: false }
      for (const view of views) {
        view.activity(shown)
      }
    }, 3000))
  }

  /** The agent opens `simulator`; a device already open keeps its tab. */
  async function openDevice(simulator: GallerySimulator): Promise<OpenDevice> {
    await beat(600)
    let shown = [...open.values()].find((each) => each.simulator === simulator)
    if (!shown) {
      numbers += 1
      shown = { id: `m${numbers}`, simulator, orientation: 'portrait', dark: false, screen: 'home', scrolled: 0, off: false, touch: null, activity: { active: false }, recording: false }
      open.set(shown.id, shown)
    }
    if (!panel.all.some((tab) => tab.kind === 'mobile' && shows(tab) === shown.id)) {
      const data = { device: shown.id, nativeId: simulator.nativeId, name: simulator.name }
      panel.apply({ type: 'create', tab: { id: `mobile-${shown.id}`, kind: 'mobile', data } })
    }
    changed()
    return shown
  }

  function closeDevice(id: string): void {
    clearTimeout(lingering.get(id))
    lingering.delete(id)
    if (open.delete(id)) {
      for (const tab of panel.all.filter((each) => each.kind === 'mobile' && shows(each) === id)) {
        panel.apply({ type: 'remove', id: tab.id })
      }
      changed()
    }
  }

  async function call(method: string, params: object): Promise<unknown> {
    switch (method) {
      case 'close': {
        const { device: id } = onDeviceSchema.parse(params)
        closeDevice(id)
        return closeResultSchema.parse({ device: id, shutDown: false })
      }
      case 'rotate': {
        const { device: id, turn } = rotateDeviceSchema.parse(params)
        const shown = device(id)
        const at = TURNS.indexOf(shown.orientation)
        shown.orientation = TURNS[(at + (turn === 'left' ? 1 : 3)) % 4] ?? 'portrait'
        restart(id)
        const size = upright(shown)
        return { device: id, orientation: shown.orientation, width: size.width * SCALE, height: size.height * SCALE }
      }
      case 'toggleAppearance': {
        const { device: id } = onDeviceSchema.parse(params)
        const shown = device(id)
        shown.dark = !shown.dark
        return { device: id, appearance: shown.dark ? 'dark' : 'light' }
      }
      case 'press': {
        const { device: id, button } = pressButtonSchema.parse(params)
        const shown = device(id)
        if (button === 'lock') {
          // The side button turns a screen that is on off, and one that is off on.
          power(shown, shown.off)
          return { device: id, key: button }
        }
        // Any other button wakes the screen too.
        power(shown, true)
        if (button === 'home' || button === 'back') {
          // Android's Back leaves Settings for the home screen.
          shown.screen = 'home'
          shown.scrolled = 0
        } else {
          // The gallery's screen draws no volume; the simulator would show its HUD.
          productWould(button === 'volume-up' ? 'The simulator’s volume goes up.' : 'The simulator’s volume goes down.')
        }
        return { device: id, key: button }
      }
      case 'screenshot': {
        const { device: id } = onDeviceSchema.parse(params)
        device(id)
        await beat(300)
        // The Host writes the picture into the conversation's scratch directory; the gallery's workspace holds one.
        return { path: `${WORKSPACE_ROOT}/assets/photo.png` }
      }
      case 'record': {
        const { device: id, action } = recordDeviceSchema.parse(params)
        const shown = device(id)
        await beat(action === 'stop' ? 600 : 200)
        return record(shown, action === 'start')
      }
    }
    throw new PluginCallError('unknown_plugin_method', `the simulators have no method ${method}`, { type: 'status', status: '404 Not Found' })
  }

  /** Starts or stops recording `shown`; every view hears it, as each window would. */
  function record(shown: OpenDevice, start: boolean): object {
    if (shown.recording === start) {
      throw new PluginCallError(start ? 'recording' : 'not_recording', 'the recording is not as asked', { type: 'status', status: '409 Conflict' })
    }
    shown.recording = start
    changed()
    return start
      ? { device: shown.id, recording: true }
      : { device: shown.id, recording: false, path: `${WORKSPACE_ROOT}/assets/demo.mp4`, durationMs: 4200 }
  }

  /** Turns `shown`'s screen on or off; every view of it hears, as the Host tells each. */
  function power(shown: OpenDevice, on: boolean): void {
    if (shown.off === !on) {
      return
    }
    shown.off = !on
    for (const view of views) {
      view.screen(shown)
    }
  }

  function restart(id: string): void {
    for (const view of views) {
      if (view.watching === id) {
        view.restart()
      }
    }
  }

  const stream: OpenUserStream = (handlers) => {
    const view = new MobileView(handlers, () => [...open.values()], (shown) => power(shown, true))
    views.add(view)
    return {
      send: (bytes) => view.receive(bytes),
      close: () => {
        views.delete(view)
        view.stop()
      },
    }
  }

  return {
    plugin: { call: (method, params) => call(method, params), streams: { mobile: stream } },
    removed: (tab) => {
      const id = shows(tab)
      if (id) {
        closeDevice(id)
      }
    },
    agentOpens: async () => {
      const next = SIMULATORS.find((simulator) => ![...open.values()].some((each) => each.simulator === simulator)) ?? SIMULATORS[0]
      if (next) {
        agentUsed(await openDevice(next))
      }
    },
    agentTaps: () => {
      for (const shown of open.values()) {
        const size = upright(shown)
        const point = { x: size.width / 2, y: size.height * 0.3 }
        shown.activity = { active: true, point }
        shown.touch = { ...point, at: performance.now() }
        for (const view of views) {
          view.activity(shown)
        }
        setTimeout(() => {
          shown.screen = 'settings'
          agentUsed(shown)
        }, 300)
      }
    },
    agentRecords: () => {
      const first = open.values().next().value
      if (!first) {
        productWould('The agent has no simulator open to record.')
        return
      }
      record(first, !first.recording)
      agentUsed(first)
    },
    devicesSleep: () => {
      if (open.size === 0) {
        productWould('No simulator is open to sleep.')
        return
      }
      for (const shown of open.values()) {
        power(shown, false)
      }
    },
  }
}

/** One page's view of the gallery's devices, as a `demi-mobile` service serves it. */
class MobileView {
  watching: string | null = null
  private encoder: VideoEncoder | null = null
  private painting: ReturnType<typeof setInterval> | null = null
  private readonly heartbeat: ReturnType<typeof setInterval>
  private readonly canvas = document.createElement('canvas')
  private generation = 0
  private sequence = 0
  private readonly started = performance.now()
  /** The finger on the watched screen. */
  private finger: { x: number; y: number } | null = null
  /** The size the view lays the frame into; the gallery's frame is the outline, whatever the size. */
  private area: { width: number; height: number } | null = null

  constructor(
    private readonly handlers: UserStreamHandlers,
    private readonly devices: () => OpenDevice[],
    private readonly wake: (device: OpenDevice) => void,
  ) {
    this.heartbeat = setInterval(() => this.send({ type: 'heartbeat' }), 250)
  }

  private send(value: LiveModuleMessage): void {
    this.handlers.data(encodeControl(value))
  }

  private device(): OpenDevice | null {
    return this.devices().find((each) => each.id === this.watching) ?? null
  }

  state(): void {
    if (this.watching !== null && !this.device()) {
      this.watching = null
      this.restart()
    }
    this.send({
      type: 'state',
      devices: this.devices().map((each) => ({
        id: each.id,
        name: each.simulator.name,
        platform: each.simulator.platform,
        os: each.simulator.os,
        recording: each.recording,
      })),
      watched: this.watching,
    })
  }

  activity(device: OpenDevice): void {
    if (device.id === this.watching) {
      this.send({ type: 'activity', device: device.id, activity: device.activity })
    }
  }

  screen(device: OpenDevice): void {
    if (device.id === this.watching) {
      this.send({ type: 'screen', device: device.id, on: !device.off })
    }
  }

  receive(bytes: Uint8Array): void {
    if (bytes.length < 5 || bytes[4] !== LIVE_CONTROL_FRAME) {
      return
    }
    const length = new DataView(bytes.buffer, bytes.byteOffset).getUint32(0)
    const value = JSON.parse(decoder.decode(bytes.subarray(5, 4 + length))) as LiveViewerMessage
    switch (value.type) {
      case 'hello':
        this.state()
        break
      case 'watch':
        this.watching = value.device
        this.area = null
        this.restart()
        this.state()
        break
      case 'input':
        this.input(value.input)
        break
      case 'wake': {
        const device = this.device()
        if (device && device.id === value.device) {
          this.wake(device)
        }
        break
      }
      case 'frame':
        if (value.device === this.watching) {
          this.area = { width: value.width, height: value.height }
          this.frame()
        }
        break
      // The gallery draws its pictures at one size, whatever the panel shows.
      case 'shown':
      case 'keyframe':
      case 'release':
        break
    }
  }

  /** The watched device's frame: no frame is installed here, so the view draws the outline. */
  private frame(): void {
    const device = this.device()
    if (!device || !this.area) {
      return
    }
    const frame: LiveFrame = {
      kind: 'outline',
      cornerRadius: device.simulator.cornerRadius,
      buttons: device.simulator.buttons.map((mark) => turned(mark, device.orientation)),
    }
    const points = upright(device)
    this.send({ type: 'frame', device: device.id, orientation: device.orientation, pointsWidth: points.width, pointsHeight: points.height, frame })
  }

  /** A tap opens Settings from the home screen; a finger draws its ring where it touches. */
  private input(input: ViewerInput): void {
    const device = this.device()
    // The wheel scrolls Settings, as a swipe of its turn would; the home screen does not scroll.
    if (device && input.type === 'wheel') {
      if (device.screen === 'settings') {
        const end = Math.max(0, SETTINGS_HEIGHT - upright(device).height)
        device.scrolled = Math.max(0, Math.min(end, device.scrolled + input.deltaY))
      }
      return
    }
    if (!device || input.type !== 'touch') {
      return
    }
    const contact = input.contacts[0]
    if (input.action === 'start' && contact) {
      this.finger = { x: contact.x, y: contact.y }
      device.touch = { x: contact.x, y: contact.y, at: performance.now() }
    } else if (input.action === 'end') {
      if (device.screen === 'home' && this.finger && this.finger.y < upright(device).height * 0.5) {
        device.screen = 'settings'
      }
      this.finger = null
    }
  }

  restart(): void {
    if (this.painting !== null) {
      clearInterval(this.painting)
      this.painting = null
    }
    if (this.encoder && this.encoder.state !== 'closed') {
      this.encoder.close()
    }
    this.encoder = null
    const device = this.device()
    if (!device) {
      return
    }
    const points = upright(device)
    // The live view's long side, as demi-mobile scales the pictures.
    const scale = Math.min(1, 1568 / (Math.max(points.width, points.height) * SCALE))
    const size = {
      width: Math.round((points.width * SCALE * scale) / 2) * 2,
      height: Math.round((points.height * SCALE * scale) / 2) * 2,
    }
    this.generation += 1
    this.sequence = 0
    this.canvas.width = size.width
    this.canvas.height = size.height
    this.send({ type: 'inputs', device: device.id, inputs: { pointer: 'touch' } })
    this.send({ type: 'activity', device: device.id, activity: device.activity })
    this.screen(device)
    this.send({
      type: 'stream',
      device: device.id,
      generation: this.generation,
      width: size.width,
      height: size.height,
      pointsWidth: points.width,
      pointsHeight: points.height,
      orientation: device.orientation,
      scale,
    })
    // A turn draws the frame again, turned.
    this.frame()
    const generation = this.generation
    this.encoder = new VideoEncoder({
      output: (chunk) => {
        if (generation !== this.generation) {
          return
        }
        this.sequence += 1
        const data = new Uint8Array(chunk.byteLength)
        chunk.copyTo(data)
        this.handlers.data(encodeVideo({
          source: device.id,
          generation,
          sequence: this.sequence,
          key: chunk.type === 'key',
          timestamp: chunk.timestamp,
          width: size.width,
          height: size.height,
          data,
        }))
      },
      error: () => this.restart(),
    })
    this.encoder.configure({
      codec: LIVE_VIDEO_CODEC,
      width: size.width,
      height: size.height,
      framerate: FPS,
      bitrate: 2_000_000,
      latencyMode: 'realtime',
      hardwareAcceleration: 'prefer-software',
      avc: { format: 'annexb' },
    })
    let frames = 0
    this.painting = setInterval(() => {
      this.paint(device, points, size.width / points.width)
      const frame = new VideoFrame(this.canvas, { timestamp: Math.round((performance.now() - this.started) * 1000) })
      try {
        this.encoder?.encode(frame, { keyFrame: frames % (FPS * 2) === 0 })
      } finally {
        frame.close()
        frames += 1
      }
    }, 1000 / FPS)
  }

  /** A phone's screen worth looking at: a home screen of app icons or Settings; a screen that is off keeps its last picture. */
  private paint(device: OpenDevice, points: { width: number; height: number }, ratio: number): void {
    const context = this.canvas.getContext('2d')
    if (!context || device.off) {
      return
    }
    context.setTransform(ratio, 0, 0, ratio, 0, 0)
    const { width, height } = points
    const background = device.dark ? '#000000' : '#f2f2f7'
    const foreground = device.dark ? '#ffffff' : '#000000'
    context.fillStyle = background
    context.fillRect(0, 0, width, height)
    context.fillStyle = foreground
    context.font = '600 15px -apple-system, system-ui, sans-serif'
    context.fillText('9:41', 32, 34)
    if (device.screen === 'home') {
      const gradient = context.createLinearGradient(0, 0, width, height)
      gradient.addColorStop(0, device.dark ? '#1c1c3a' : '#7ab8ff')
      gradient.addColorStop(1, device.dark ? '#3a1c32' : '#f7a8d8')
      context.fillStyle = gradient
      context.fillRect(0, 50, width, height - 50)
      const colors = ['#34c759', '#0a84ff', '#ff9f0a', '#8e8e93', '#ff375f', '#5e5ce6', '#64d2ff', '#ffd60a']
      const columns = 4
      const cell = width / columns
      colors.forEach((color, index) => {
        const x = (index % columns) * cell + cell / 2 - 30
        const y = 90 + Math.floor(index / columns) * 96
        context.fillStyle = color
        context.beginPath()
        context.roundRect(x, y, 60, 60, 14)
        context.fill()
      })
    } else {
      // The list scrolls under the status bar.
      context.save()
      context.beginPath()
      context.rect(0, 50, width, height - 50)
      context.clip()
      context.translate(0, -device.scrolled)
      context.font = '700 32px -apple-system, system-ui, sans-serif'
      context.fillText('Settings', 20, 104)
      let top = SETTINGS_TOP
      for (const group of SETTINGS_GROUPS) {
        context.fillStyle = device.dark ? '#1c1c1e' : '#ffffff'
        context.beginPath()
        context.roundRect(16, top, width - 32, group.length * SETTINGS_ROW, 12)
        context.fill()
        context.fillStyle = foreground
        context.font = '17px -apple-system, system-ui, sans-serif'
        group.forEach((label, index) => {
          context.fillText(label, 32, top + 30 + index * SETTINGS_ROW)
        })
        top += group.length * SETTINGS_ROW + SETTINGS_GAP
      }
      context.restore()
    }
    const touch = device.touch
    if (touch && performance.now() - touch.at < 800) {
      context.strokeStyle = 'rgba(10, 132, 255, 0.9)'
      context.lineWidth = 3
      context.beginPath()
      context.arc(touch.x, touch.y, 18, 0, Math.PI * 2)
      context.stroke()
    }
    void height
  }

  stop(): void {
    clearInterval(this.heartbeat)
    if (this.painting !== null) {
      clearInterval(this.painting)
    }
    if (this.encoder && this.encoder.state !== 'closed') {
      this.encoder.close()
    }
    this.encoder = null
  }
}
