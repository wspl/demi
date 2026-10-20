/**
 * One live view of the conversation's simulators
 * (`crates/plugin-mobile/page/src/index.ts`): the page's side of the `mobile` stream,
 * over the live view's link that the browser's view uses too.
 *
 * The session:
 *
 * - Keeps what the view shows.
 * - Sends the viewer's touches and keys.
 * - Follows the agent's activity on the watched device, and keeps each open
 *   device's last frame with its last picture, so a return to its tab shows
 *   both at once until a newer frame replaces it.
 * - Watches a device only while one of its tabs is shown in a visible page, so a
 *   device nobody looks at gets its hour; with none shown, the view stays open
 *   watching nothing and says nobody watches it, so nothing stays awake for it,
 *   and a return watches at once on the same connection.
 * - Keeps each device's last picture as the view that unmounts hands it on,
 *   and each frame the module draws in the plugin's disk cache, by the
 *   device's model, the size and pixel ratio it is drawn for and its
 *   orientation, so a device shows its frame at once, even after a reload.
 * - Turns a device's shown picture and frame at once when the viewer rotates
 *   it, until a picture drawn in the orientation the device reports replaces
 *   them, and turns them back if the device refuses
 *   (`decisions/0007-rotation-shows-at-once.md`).
 */
import { reactive } from 'vue'
import { z } from 'zod'
import {
  LiveFrameReader,
  LiveLink,
  encodeControl,
  type LiveActivity,
  type LiveConnection,
  type LiveRawFrame,
  type OpenUserStream,
  type PictureSink as LivePictureSink,
  type PluginCache,
  type ViewerInput,
} from '@demicodes/plugin-sdk'
import {
  LIVE_CAPTURE_FAILED,
  LIVE_CAPTURE_UNAVAILABLE,
  liveFrameSchema,
  liveModuleMessageSchema,
  liveViewerMessageSchema,
  orientationSchema,
  panelReasonSchema,
  type LiveDevice,
  type LiveFrame,
  type LiveModuleMessage,
  type LiveViewerMessage,
  type MobilePlatform,
  type Orientation,
  type PanelReason,
  type Turn,
} from '../generated/plugin'

export { REFUSED_FRAME, SILENT_STREAM, type LiveConnection } from '@demicodes/plugin-sdk'

/** A generation of the watched device's pictures, as the module announced it. */
export interface DeviceStream {
  device: string
  generation: number
  /** The picture's size in pixels. */
  width: number
  height: number
  /** The screen's size in points, as the picture shows it, which input is given in. */
  pointsWidth: number
  pointsHeight: number
  orientation: Orientation
}

export type PictureSink = LivePictureSink<DeviceStream>

/** The notices that leave the watched device without a picture. */
export type PicturelessReason = typeof LIVE_CAPTURE_UNAVAILABLE | typeof LIVE_CAPTURE_FAILED

/** A device's frame, for the orientation it was drawn for. */
export interface DeviceFrame {
  device: string
  orientation: Orientation
  /** The screen's size in points, turned as the frame is. */
  pointsWidth: number
  pointsHeight: number
  /** None while the device still reads what its frame needs; the picture shows without one meanwhile. */
  frame: LiveFrame | null
}

/**
 * A press of Rotate Left or Right on a device, until the device has shown
 * its effect: its identity, its quarter turn, the orientation it turns the
 * shown device to, and once the device answered, the orientation it answered
 * with.
 */
export interface DeviceTurn {
  id: number
  device: string
  quarters: number
  expected: Orientation
  answered: Orientation | null
  /** The orientations the device's pictures came in since the press. */
  reported: Orientation[]
}

/** Each orientation's quarter turns clockwise from portrait, as the device turns. */
const QUARTERS: Record<Orientation, number> = {
  'portrait': 0,
  'landscape-right': 1,
  'portrait-upside-down': 2,
  'landscape-left': 3,
}
const ORIENTATIONS: readonly Orientation[] = ['portrait', 'landscape-right', 'portrait-upside-down', 'landscape-left']

/** `orientation` turned by `quarters` clockwise. */
function turned(orientation: Orientation, quarters: number): Orientation {
  return ORIENTATIONS[(((QUARTERS[orientation] + quarters) % 4) + 4) % 4]!
}

/** The quarter turns, clockwise, from `from` to `to`, the shorter way: -1 to 2. */
export function quartersBetween(from: Orientation, to: Orientation): number {
  const quarters = (((QUARTERS[to] - QUARTERS[from]) % 4) + 4) % 4
  return quarters === 3 ? -1 : quarters
}

const TURN_QUARTERS: Record<Turn, number> = { left: -1, right: 1 }

export interface LiveState {
  connection: LiveConnection
  /** The conversation's open devices on this Host, each with whether it records. */
  devices: LiveDevice[]
  watched: string | null
  /** The agent's activity on the watched device. */
  activity: LiveActivity
  /**
   * Each open device's last frame, as the module drew it for the view's
   * size while the device was watched; a newer one replaces it.
   */
  frames: Record<string, DeviceFrame>
  /** Whether the watched device's screen is off, as the Host says. */
  screenOff: boolean
  pictureless: PicturelessReason | null
  ended: string | null
  /** The viewer's presses of Rotate Left or Right whose effect the device has not reported yet. */
  turning: DeviceTurn[]
  /** Each device's orientation, as its last announced pictures report it. */
  orientations: Record<string, Orientation>
}

/** A device's last picture, copied off the canvas that showed it, with the generation it was of, which places it. */
export interface KeptPicture {
  image: CanvasImageSource & { width: number; height: number }
  stream: DeviceStream
}

/** A frame as the disk cache keeps it, read back against this schema. */
const cachedFrameSchema = z.object({
  orientation: orientationSchema,
  pointsWidth: z.number(),
  pointsHeight: z.number(),
  frame: liveFrameSchema,
})

export interface MobileSessionOptions {
  open: OpenUserStream
  /** The plugin's disk cache, which keeps the frames the module draws; none keeps none. */
  cache?: PluginCache
  platform: 'mac' | 'windows' | 'linux' | 'other'
  /**
   * A notice the picture still shows through, such as input that did not
   * arrive, with the notice's message, which only a reason the page quotes
   * from, such as the viewer's own text, is meant for.
   */
  onNotice?: (reason: PanelReason, message: string) => void
  defect: (message: string, error: unknown) => void
  now?: () => number
}

const IDLE: LiveActivity = { active: false }

type Frame = { kind: 'message'; message: LiveModuleMessage } | Extract<LiveRawFrame, { kind: 'video' }>

/** The device pixels the view lays a frame into, `scale` of them per CSS pixel, as a request of a frame names them. */
interface FrameArea {
  width: number
  height: number
  scale: number
}

/** JSON is UTF-8, so a control message that is not is refused rather than repaired. */
const strictDecoder = new TextDecoder('utf-8', { fatal: true })

/** The `mobile` stream's frames, its control messages checked against the protocol. */
class DeviceFrames {
  private readonly raw = new LiveFrameReader()

  read(chunk: Uint8Array): Frame[] {
    return this.raw.read(chunk).map((frame) => {
      if (frame.kind === 'video') {
        return frame
      }
      const parsed = liveModuleMessageSchema.safeParse(JSON.parse(strictDecoder.decode(frame.payload)))
      if (!parsed.success) {
        throw new Error('the live stream sent a control message the protocol refuses', { cause: parsed.error })
      }
      return { kind: 'message', message: parsed.data }
    })
  }
}

export class MobileSession {
  readonly state: LiveState = reactive({
    connection: 'opening',
    devices: [],
    watched: null,
    activity: IDLE,
    frames: {},
    screenOff: false,
    pictureless: null,
    ended: null,
    turning: [],
    orientations: {},
  })

  private readonly link: LiveLink<Frame, DeviceStream>
  /** Whether this web browser shows the pictures, so a watch may open the stream. */
  private enabled = false
  /** The devices whose tabs are shown now, the latest last. */
  private readonly shown: string[] = []
  /** Whether the page is visible; a hidden page watches nothing. */
  private visible = true
  /** Whether a settling of the watch is queued for after this turn's tab changes. */
  private settling = false
  /** The device pixels the view lays the frame into, as it last measured them. */
  private area: FrameArea | null = null
  /** The device pixels the view shows the screen at, as it last measured them. */
  private screenSize: { width: number; height: number } | null = null
  /** The identity the next press of Rotate Left or Right takes. */
  private nextTurn = 1
  /** Each open device's last picture, which a view of it starts with. */
  private readonly pictures = new Map<string, KeptPicture>()
  /** The disk cache's key of the frame being read, so a key is read once at a time. */
  private readingFrame: string | null = null

  constructor(private readonly options: MobileSessionOptions) {
    this.link = new LiveLink(this.state, {
      open: options.open,
      frames: () => new DeviceFrames(),
      opened: () => {
        this.send({ type: 'hello', platform: options.platform })
        if (this.state.watched) {
          this.send({ type: 'watch', device: this.state.watched })
          this.askFrame()
          this.tellShown()
        }
      },
      received: (frame) => this.receive(frame),
      ended: () => {
        // The next connection's pictures report the devices' orientations anew.
        this.state.orientations = {}
        this.state.pictureless = null
        this.state.activity = IDLE
        this.state.screenOff = false
        this.state.devices = []
      },
      stalled: () => this.release(),
      watching: (stream) => stream.device === this.state.watched,
      keyframe: (stream) => this.send({ type: 'keyframe', generation: stream.generation }),
      defect: options.defect,
      now: options.now,
    })
  }

  /** The platform of `device`, while it is open. */
  platform(device: string): MobilePlatform | null {
    return this.state.devices.find((listed) => listed.id === device)?.platform ?? null
  }

  /** Whether `device` records now, as the device says, whoever started it. */
  recording(device: string): boolean {
    return this.state.devices.some((listed) => listed.id === device && listed.recording)
  }

  /** The generation the pictures come in now, for the device the view watches. */
  get picture(): DeviceStream | null {
    return this.link.picture
  }

  attach(pictures: PictureSink): void {
    this.link.attach(pictures)
  }

  detach(pictures: PictureSink): void {
    this.link.detach(pictures)
  }

  start(): void {
    this.link.start()
  }

  close(): void {
    this.link.close()
  }

  private send(message: LiveViewerMessage): void {
    const checked = liveViewerMessageSchema.safeParse(message)
    if (!checked.success) {
      this.options.defect(`The live view built an invalid ${message.type} message`, checked.error)
      return
    }
    this.link.send(encodeControl(checked.data))
  }

  private receive(frame: Frame): void {
    if (frame.kind === 'video') {
      if (this.link.show(frame.frame)) {
        this.state.pictureless = null
      }
      return
    }
    const message = frame.message
    switch (message.type) {
      case 'state':
        this.link.live()
        this.state.devices = message.devices
        // A device that closed takes its frame, its picture and its turn along.
        for (const device of Object.keys(this.state.frames)) {
          if (!message.devices.some((listed) => listed.id === device)) {
            delete this.state.frames[device]
          }
        }
        for (const device of [...this.pictures.keys()]) {
          if (!message.devices.some((listed) => listed.id === device)) {
            this.pictures.delete(device)
          }
        }
        this.state.turning = this.state.turning.filter((turn) => message.devices.some((listed) => listed.id === turn.device))
        // The watched device's model is known now, which its cached frame is kept by.
        this.cachedFrame()
        for (const device of Object.keys(this.state.orientations)) {
          if (!message.devices.some((listed) => listed.id === device)) {
            delete this.state.orientations[device]
          }
        }
        if (this.state.watched !== message.watched) {
          const wanted = this.state.watched
          if (wanted !== null && message.devices.some((device) => device.id === wanted)) {
            this.send({ type: 'watch', device: wanted })
          } else {
            this.state.watched = message.watched
          }
        }
        break
      case 'stream':
        this.reported(message.device, message.orientation)
        this.link.generation({
          device: message.device,
          generation: message.generation,
          width: message.width,
          height: message.height,
          pointsWidth: message.pointsWidth,
          pointsHeight: message.pointsHeight,
          orientation: message.orientation,
        })
        // The device's orientation is known now, which its cached frame is kept by.
        this.cachedFrame()
        break
      case 'activity':
        if (message.device === this.state.watched) {
          this.state.activity = message.activity
        }
        break
      case 'frame':
        if (message.device === this.state.watched) {
          this.state.frames[message.device] = {
            device: message.device,
            orientation: message.orientation,
            pointsWidth: message.pointsWidth,
            pointsHeight: message.pointsHeight,
            frame: message.frame,
          }
          if (message.frame !== null) {
            const area = { width: message.width, height: message.height, scale: message.scale }
            this.cacheFrame(message.device, area, message.orientation, {
              orientation: message.orientation,
              pointsWidth: message.pointsWidth,
              pointsHeight: message.pointsHeight,
              frame: message.frame,
            })
          }
        }
        break
      case 'screen':
        if (message.device === this.state.watched) {
          this.state.screenOff = !message.on
        }
        break
      case 'inputs':
      case 'heartbeat':
        break
      case 'notice': {
        const reason = panelReasonSchema.safeParse(message.code)
        if (!reason.success) {
          this.options.defect('The live view sent a notice of a reason the page does not know', reason.error)
        } else if (reason.data === LIVE_CAPTURE_FAILED || reason.data === LIVE_CAPTURE_UNAVAILABLE) {
          this.state.pictureless = reason.data
        } else {
          this.options.onNotice?.(reason.data, message.message)
        }
        break
      }
    }
  }

  resync(): void {
    this.link.resync()
  }

  /**
   * The viewer pressed Rotate Left or Right on `device`: the press, until
   * the device has shown its effect, turns the device's shown picture and
   * frame a quarter at once. Answers the press's identity, which its answer
   * names; none when the device has reported no orientation yet to turn
   * from.
   */
  turning(device: string, turn: Turn): number | null {
    const shown = this.shownOrientation(device)
    if (!shown) {
      return null
    }
    const id = this.nextTurn++
    const quarters = TURN_QUARTERS[turn]
    this.state.turning.push({ id, device, quarters, expected: turned(shown, quarters), answered: null, reported: [] })
    return id
  }

  /**
   * The device answered the press `id` with `orientation`, or refused it
   * (none), which takes the press back. A press whose answer the device has
   * reported since it was pressed is done; an answer to a press already done
   * or never kept changes nothing.
   */
  turnAnswered(id: number | null, orientation: Orientation | null): void {
    const turn = this.state.turning.find((each) => each.id === id)
    if (!turn) {
      return
    }
    if (orientation === null || turn.reported.includes(orientation)) {
      this.state.turning = this.state.turning.filter((each) => each !== turn)
      return
    }
    turn.answered = orientation
  }

  /**
   * `device`'s pictures come in `orientation`, as it announced them,
   * whoever turned it: a press it answered with that orientation, or one
   * not answered yet that turns the device to it, is done, the device now
   * showing it, and so is every answered press before it, which the device
   * has turned past. A turn by someone else that ends where a press aims
   * ends the press too; the device's further turns show as it reports them.
   */
  private reported(device: string, orientation: Orientation): void {
    this.state.orientations[device] = orientation
    const presses = this.state.turning.filter((turn) => turn.device === device)
    for (const turn of presses) {
      turn.reported.push(orientation)
    }
    const shows = (turn: DeviceTurn) => (turn.answered ?? turn.expected) === orientation
    const last = presses.findLastIndex(shows)
    const done = new Set(presses.filter((turn, at) => shows(turn) || (at < last && turn.answered !== null)))
    this.state.turning = this.state.turning.filter((turn) => !done.has(turn))
  }

  /**
   * The orientation `device`'s picture is shown in: the one it reports,
   * turned by the viewer's presses whose effect it has not reported, answered
   * or not; none before it reports one.
   */
  shownOrientation(device: string): Orientation | null {
    const reported = this.state.orientations[device]
    if (!reported) {
      return null
    }
    const quarters = this.state.turning
      .filter((turn) => turn.device === device)
      .reduce((sum, turn) => sum + turn.quarters, 0)
    return turned(reported, quarters)
  }

  /** The quarter turns, clockwise, the view adds to `drawn`, the orientation of its device's picture drawn now. */
  pendingQuarters(device: string, drawn: Orientation): number {
    const shown = this.shownOrientation(device)
    return shown ? quartersBetween(drawn, shown) : 0
  }

  /** The last picture a view of `device` showed, kept for the next view of it. */
  keepPicture(device: string, picture: KeptPicture): void {
    if (this.state.devices.some((listed) => listed.id === device)) {
      this.pictures.set(device, picture)
    }
  }

  /** The device's last picture, if a view kept one. */
  keptPicture(device: string): KeptPicture | null {
    return this.pictures.get(device) ?? null
  }

  /**
   * The disk cache's key of a frame of `device` in `orientation`, drawn for
   * `area`: none for a device whose model the driver does not know, whose
   * frame another device of no model may not share.
   */
  private frameKey(device: string, area: FrameArea, orientation: Orientation): string | null {
    const model = this.state.devices.find((listed) => listed.id === device)?.model
    if (!model) {
      return null
    }
    return `frame:${model}:${area.width}x${area.height}@${area.scale}:${orientation}`
  }

  /**
   * The disk cache's key of the frame the watched device shows now: for the
   * area the view last measured and the orientation the device shows.
   */
  private watchedFrameKey(): { device: string; orientation: Orientation; key: string } | null {
    const device = this.state.watched
    if (device === null || this.area === null) {
      return null
    }
    const orientation = this.frameOrientation(device)
    const key = orientation === null ? null : this.frameKey(device, this.area, orientation)
    return orientation === null || key === null ? null : { device, orientation, key }
  }

  /**
   * The orientation `device`'s frame is drawn in: the one its picture is
   * shown in, or before its pictures report one, the one its screen reports
   * in the state. The pictures start only once the view laid the screen out,
   * for which it needs a frame, so a cached frame cannot wait for them;
   * turns still go by the pictures alone.
   */
  private frameOrientation(device: string): Orientation | null {
    return this.shownOrientation(device) ?? this.state.devices.find((listed) => listed.id === device)?.orientation ?? null
  }

  /** Keeps a frame the module drew for `area`, the area of the request it answers, which the view may have left since. */
  private cacheFrame(device: string, area: FrameArea, orientation: Orientation, frame: z.infer<typeof cachedFrameSchema>): void {
    const key = this.frameKey(device, area, orientation)
    const cache = this.options.cache
    if (key === null || !cache) {
      return
    }
    cache.write(key, new TextEncoder().encode(JSON.stringify(frame))).catch((error: unknown) => {
      // A cache that cannot be written leaves the frame to come from the Host again.
      this.options.defect('The device frame could not be cached', error)
    })
  }

  /**
   * The watched device's frame from the disk cache, for the orientation it
   * shows, until the module's own arrives, once the view's size, the device's
   * model and its orientation are known; a frame of that orientation the view
   * already has stays. A frame read for a device, model, size or orientation
   * the view no longer shows is not shown.
   */
  private cachedFrame(): void {
    const wanted = this.watchedFrameKey()
    const cache = this.options.cache
    if (wanted === null || !cache || wanted.key === this.readingFrame || this.state.frames[wanted.device]?.orientation === wanted.orientation) {
      return
    }
    const { device, orientation, key } = wanted
    this.readingFrame = key
    cache.read(key).then((bytes) => {
      if (this.readingFrame === key) {
        this.readingFrame = null
      }
      const current = this.watchedFrameKey()
      if (bytes === null || current?.device !== device || current.key !== key || this.state.frames[device]?.orientation === orientation) {
        return
      }
      const kept = cachedFrameSchema.safeParse(JSON.parse(new TextDecoder().decode(bytes)))
      if (!kept.success) {
        // Written by another build of the page: the Host's own frame follows.
        return
      }
      this.state.frames[device] = { device, ...kept.data }
    }, (error: unknown) => {
      if (this.readingFrame === key) {
        this.readingFrame = null
      }
      this.options.defect('The device frame could not be read from the cache', error)
    })
  }

  /** Without a byte for a second the connection is stalled, and what the viewer holds is let go. */
  tick(): void {
    this.link.tick()
  }

  /**
   * This web browser shows the pictures: the stream opens once a device
   * tab is shown, as the browser's view does, and at once when one already is.
   */
  enable(): void {
    this.enabled = true
    this.settle()
  }

  /** `device`'s tab is shown: the view watches it. */
  showing(device: string): void {
    this.forget(device)
    this.shown.push(device)
    this.settle()
  }

  /**
   * `device`'s tab is hidden or gone. The view lets the device go once the
   * turn's other changes are in, so a switch to another device's tab, whose
   * show may come before or after this, watches that device without a
   * moment of watching nothing.
   */
  hiding(device: string): void {
    this.forget(device)
    if (this.settling) {
      return
    }
    this.settling = true
    queueMicrotask(() => {
      this.settling = false
      this.settle()
    })
  }

  /** The page became visible or hidden; a hidden page watches nothing. */
  visibility(visible: boolean): void {
    this.visible = visible
    this.settle()
  }

  private forget(device: string): void {
    const index = this.shown.indexOf(device)
    if (index >= 0) {
      this.shown.splice(index, 1)
    }
  }

  /** Watches the device of the tab shown last, or nothing when no tab is shown in a visible page. */
  private settle(): void {
    const wanted = this.visible ? this.shown.at(-1) ?? null : null
    if (wanted === this.state.watched) {
      if (wanted !== null && this.enabled && !this.link.active) {
        this.start()
      }
      return
    }
    this.watch(wanted)
  }

  private watch(device: string | null): void {
    this.state.watched = device
    this.state.activity = IDLE
    this.state.pictureless = null
    this.state.screenOff = false
    // The view of the device now watched says the size it shows it at.
    this.screenSize = null
    this.link.unwatched()
    const active = this.link.active
    // Watched or not, as the stream says (`LiveLink.watched`); a view that
    // ended while nobody watched it opens again here.
    this.link.watched(device !== null)
    if (!active) {
      if (device !== null && this.enabled && !this.link.active) {
        this.start()
      }
      return
    }
    this.send({ type: 'watch', device })
    this.askFrame()
  }

  /**
   * The view lays the watched device's frame into `width` × `height` device
   * pixels, `scale` of them per CSS pixel: the module draws it for that size
   * and scale, and again when the device turns. The view says so once its
   * size settled.
   */
  frame(width: number, height: number, scale: number): void {
    const area = { width: Math.max(1, Math.round(width)), height: Math.max(1, Math.round(height)), scale }
    if (this.area && this.area.width === area.width && this.area.height === area.height && this.area.scale === area.scale) {
      return
    }
    this.area = area
    this.askFrame()
  }

  private askFrame(): void {
    if (this.area && this.state.watched !== null) {
      this.cachedFrame()
      this.send({ type: 'frame', device: this.state.watched, width: this.area.width, height: this.area.height, scale: this.area.scale })
    }
  }

  /**
   * The view shows the watched device's screen at `width` × `height` device
   * pixels: the Host captures it no larger (`crates/plugin-mobile/page/src/index.ts`).
   */
  showsScreenAt(width: number, height: number): void {
    const size = { width: Math.max(1, Math.round(width)), height: Math.max(1, Math.round(height)) }
    if (this.screenSize && this.screenSize.width === size.width && this.screenSize.height === size.height) {
      return
    }
    this.screenSize = size
    this.tellShown()
  }

  private tellShown(): void {
    if (this.screenSize && this.state.watched !== null && this.link.active) {
      this.send({ type: 'shown', device: this.state.watched, ...this.screenSize })
    }
  }

  /** Wake: the watched device's screen turns on, and never off (`crates/plugin-mobile/page/src/index.ts`). */
  wake(): void {
    const device = this.state.watched
    if (device !== null) {
      this.send({ type: 'wake', device })
    }
  }

  /** The viewer's touch or key on the watched device; dropped while the stream stalls. */
  input(input: ViewerInput): void {
    const device = this.state.watched
    if (device === null || this.state.connection !== 'live') {
      return
    }
    this.send({ type: 'input', device, input })
  }

  release(): void {
    this.send({ type: 'release' })
  }
}
