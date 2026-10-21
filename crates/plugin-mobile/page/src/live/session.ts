/**
 * One open device's live view on the page (`crates/plugin-mobile/page/src/index.ts`):
 * its own `mobile` stream, over the live view's link that the browser's view
 * uses too, and its own decoder and canvas, which the plugin's store keeps
 * while the device is open
 * (`docs/decisions/0017-each-device-streams-on-its-own.md`).
 *
 * - The stream's one state of its own is whether it is looked at: whether a
 *   view of the device shows in a visible page. While it is, the Host sends
 *   the device's pictures; while it is not, the stream stays open and the
 *   Host sends nothing. Nothing here says which device is wanted: any number
 *   of devices stream at once, each on its own session.
 * - What a view shows of the device is one look: its frame, the screen's
 *   size, its orientation and the picture painted on its canvas, which change
 *   together. A picture paints only once its orientation's frame is ready,
 *   its images decoded, so the screen and its frame always show together.
 * - Each frame names the orientation and the screen it was drawn for, as the
 *   Host drew it, and is kept for that orientation only, once its images
 *   decoded and its screen has that orientation's shape. Before the first
 *   picture, the look is the frame of the orientation the device has, and
 *   the Host is asked again while it has sent none of that orientation; a
 *   frame that cannot be shown says so and can be asked for again.
 * - Each frame the module draws is kept in the plugin's disk cache, by the
 *   device's model, the size and pixel ratio it is drawn for and its
 *   orientation, with the device's last orientation, so the device shows its
 *   frame at once, before the Host answers, even after a reload; the Host is
 *   asked only for a frame the cache lacks.
 * - A stream that ended opens again, and its view says it reconnects until
 *   it does, over the picture it last showed; the device's tab stays, as
 *   the Host keeps the device across its service's restarts.
 * - A view shown again asks its stream to answer, which the Host does with
 *   the device's state: a stream that does not answer in time, as one whose
 *   connection was lost silently while nobody looked, opens again.
 * - Turns what the view shows at once when the viewer rotates the device,
 *   from the orientation it shows, until a picture drawn in the orientation
 *   the device reports replaces it, and back if the device refuses
 *   (`docs/decisions/0007-rotation-shows-at-once.md`).
 * - Sends the viewer's touches and keys, and follows the agent's activity on
 *   the device.
 */
import { reactive } from 'vue'
import { z } from 'zod'
import {
  CanvasPictures,
  LiveFrameReader,
  LiveLink,
  encodeControl,
  type LiveActivity,
  type LiveConnection,
  type LiveRawFrame,
  type OpenUserStream,
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

/** A generation of the device's pictures, as the module announced it. */
export interface DeviceStream {
  generation: number
  /** The picture's size in pixels. */
  width: number
  height: number
  /** The screen's size in points, as the picture shows it, which input is given in. */
  pointsWidth: number
  pointsHeight: number
  orientation: Orientation
}

/** The device pixels the view lays a frame into, `scale` of them per CSS pixel, as a request of a frame names them. */
export interface FrameArea {
  width: number
  height: number
  scale: number
}

/** A device's frame for one orientation, its images decoded, ready to show. */
export interface DeviceFrame {
  orientation: Orientation
  /** The screen's size in points, turned as the frame is. */
  pointsWidth: number
  pointsHeight: number
  frame: LiveFrame
  /** The area it was drawn for. */
  area: FrameArea
}

/**
 * What a view shows of the device, whole: its frame and the screen's size
 * in an orientation, and the generation of the picture its canvas shows,
 * none before the first, when the screen shows empty in its frame.
 */
export interface DeviceLook {
  orientation: Orientation
  pointsWidth: number
  pointsHeight: number
  frame: LiveFrame
  picture: DeviceStream | null
}

/** The notices that leave the device without a picture. */
export type PicturelessReason = typeof LIVE_CAPTURE_UNAVAILABLE | typeof LIVE_CAPTURE_FAILED

/**
 * What a view of the device waits for, from the steps that run: the
 * stream, or its return after it ended, the frame, the first picture; or
 * that the frame it waits for could not be shown.
 */
export type DeviceWait = 'connecting' | 'reconnecting' | 'frame' | 'screen' | 'frameFailed' | null

/**
 * A press of Rotate Left or Right, until the device has shown its effect:
 * its identity, its quarter turn, the orientation it turns the shown device
 * to, and once the device answered, the orientation it answered with.
 */
export interface DeviceTurn {
  id: number
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

export interface DeviceState {
  connection: LiveConnection
  /** Why the stream ended, until one works again. */
  ended: string | null
  /** The device as the Host says it is, once its stream was looked at. */
  device: LiveDevice | null
  /** The agent's activity on the device. */
  activity: LiveActivity
  /** Whether the device's screen is off, as the Host says. */
  screenOff: boolean
  pictureless: PicturelessReason | null
  /** The device's orientation, as its last announced pictures on this connection report it. */
  reported: Orientation | null
  /** The device's frame of each orientation, ready to show; a newer one of the same orientation replaces it. */
  frames: Partial<Record<Orientation, DeviceFrame>>
  look: DeviceLook | null
  /** Whether the frame the view needs could not be shown, until it is asked for again. */
  frameFailed: boolean
  /** The viewer's presses of Rotate Left or Right whose effect the device has not reported yet. */
  turning: DeviceTurn[]
  /** Whether the device closed: the session has ended. */
  gone: boolean
}

/** A frame as the disk cache keeps it, read back against this schema. */
const cachedFrameSchema = z.object({
  orientation: orientationSchema,
  pointsWidth: z.number(),
  pointsHeight: z.number(),
  frame: liveFrameSchema,
})

/** A frame's images, decoded before the frame shows, so it never shows without them. */
async function decodeImages(frame: LiveFrame, decode: (src: string) => Promise<void>): Promise<void> {
  if (frame.kind !== 'image') {
    return
  }
  const sources = [`data:image/png;base64,${frame.png}`]
  if (frame.silhouette) {
    sources.push(`data:image/png;base64,${frame.silhouette}`)
  }
  await Promise.all(sources.map(decode))
}

/** Decodes an image as the page shows it. */
function decodeImage(src: string): Promise<void> {
  const image = new Image()
  image.src = src
  return image.decode()
}

export interface DeviceSessionOptions {
  open: OpenUserStream
  /** The device the stream carries. */
  device: string
  /** The device's model and native id as its tab keeps them, which find its cached frame before the Host answers. */
  model: string | null
  nativeId: string | null
  /** The plugin's disk cache, which keeps the frames the module draws; none keeps none. */
  cache?: PluginCache
  platform: 'mac' | 'windows' | 'linux' | 'other'
  /**
   * A notice the picture still shows through, such as input that did not
   * arrive, with the notice's message, which only a reason the page quotes
   * from, such as the viewer's own text, is meant for.
   */
  onNotice?: (reason: PanelReason, message: string) => void
  /** The device closed: the session has ended. */
  gone?: () => void
  defect: (message: string, error: unknown) => void
  /** Readies an image before a frame shows it; the page's own decoding unless a test says. */
  decodeImage?: (src: string) => Promise<void>
  now?: () => number
}

const IDLE: LiveActivity = { active: false }

type Frame = { kind: 'message'; message: LiveModuleMessage } | Extract<LiveRawFrame, { kind: 'video' }>

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

/**
 * Whether a drawn frame's screen has the shape of the screen it names: a
 * frame whose screen lies across for an upright screen, as a frame drawn
 * for another orientation would, never shows.
 */
function fitsItsScreen(frame: DeviceFrame): boolean {
  if (frame.frame.kind !== 'image') {
    return true
  }
  const { screen } = frame.frame
  return (screen.width > screen.height) === (frame.pointsWidth > frame.pointsHeight)
}

const sameArea = (one: FrameArea | null | undefined, other: FrameArea | null | undefined) =>
  !!one && !!other && one.width === other.width && one.height === other.height && one.scale === other.scale

export class DeviceSession {
  readonly state: DeviceState = reactive({
    connection: 'opening',
    ended: null,
    device: null,
    activity: IDLE,
    screenOff: false,
    pictureless: null,
    reported: null,
    frames: {},
    look: null,
    frameFailed: false,
    turning: [],
    gone: false,
  })

  private readonly link: LiveLink<Frame, DeviceStream>
  /** Whether this web browser shows the pictures, so the stream may open. */
  private enabled = false
  /** Whether the page is visible. */
  private visible = true
  /** The views of the device that show now. */
  private readonly views = new Set<object>()
  /** Whether the Host was told that the stream is looked at. */
  private looked = false
  /** The device pixels the view lays the frame into, as it last measured them. */
  private area: FrameArea | null = null
  /**
   * The area the Host was last asked to draw a frame for, on this
   * connection, and the orientation it answered with, or the one the view
   * needed while no answer came, if it knew it.
   */
  private asked: { area: FrameArea; orientation: Orientation | null } | null = null
  /** The device pixels the view shows the screen at, as it last measured them. */
  private screenSize: { width: number; height: number } | null = null
  /** The device's last orientation, as the disk cache keeps it. */
  private cachedOrientation: Orientation | null = null
  /** Whether the cached orientation was read. */
  private readOrientation = false
  /** The frame of each orientation being readied, the latest of each the one that shows. */
  private readonly readying = new Map<Orientation, LiveFrame>()
  /** The disk cache's keys being read, each read once at a time. */
  private readonly reading = new Set<string>()
  /** The identity the next press of Rotate Left or Right takes. */
  private nextTurn = 1
  private canvasElement: HTMLCanvasElement | null = null
  private pictures: CanvasPictures<DeviceStream> | null = null

  constructor(private readonly options: DeviceSessionOptions) {
    this.link = new LiveLink(this.state, {
      open: options.open,
      frames: () => new DeviceFrames(),
      opened: () => {
        // A new connection's Host knows nothing the last one was told.
        this.asked = null
        this.send({ type: 'hello', platform: options.platform, device: options.device })
        if (this.looked) {
          this.send({ type: 'looked', looked: true })
          this.link.expectAnswer()
          this.tellShown()
          this.seekFrames()
        }
      },
      received: (frame) => this.receive(frame),
      ended: () => {
        // The next connection's pictures report the device's orientation anew;
        // what the view shows stays until they do.
        this.state.reported = null
        this.state.pictureless = null
        this.state.activity = IDLE
        this.state.screenOff = false
      },
      stalled: () => this.release(),
      watching: () => true,
      keyframe: (stream) => this.send({ type: 'keyframe', generation: stream.generation }),
      defect: options.defect,
      now: options.now,
    })
    // A stream starts not looked at.
    this.link.watched(false)
  }

  /** The device the session's stream carries. */
  get device(): string {
    return this.options.device
  }

  /** The device's platform, once the Host said it. */
  get platform(): MobilePlatform | null {
    return this.state.device?.platform ?? null
  }

  /** Whether the device records now, as the device says, whoever started it. */
  get recording(): boolean {
    return this.state.device?.recording ?? false
  }

  /** The device's model: the Host's once it said, the tab's until then. */
  get model(): string | null {
    return this.state.device ? this.state.device.model : this.options.model
  }

  /** What a view of the device waits for, from the steps that run; none once it shows a picture. */
  get waiting(): DeviceWait {
    if (this.state.frameFailed) {
      return 'frameFailed'
    }
    if (this.state.look?.picture) {
      return null
    }
    if (this.state.connection !== 'live') {
      return this.reconnecting ? 'reconnecting' : 'connecting'
    }
    return this.state.look ? 'screen' : 'frame'
  }

  /**
   * Whether the stream ended, as with the runner's restart or a dropped
   * connection, and opens again: the device stays, and its view keeps its
   * last picture until the stream returns
   * (`docs/decisions/0031-a-device-tab-lasts-as-long-as-the-device.md`).
   */
  get reconnecting(): boolean {
    return this.state.connection === 'opening' && this.state.ended !== null
  }

  /**
   * The canvas the device's pictures paint on, which a view of it shows;
   * made, with its decoder, when a view first asks.
   */
  get canvas(): HTMLCanvasElement {
    if (!this.canvasElement) {
      this.canvasElement = document.createElement('canvas')
      this.pictures = new CanvasPictures<DeviceStream>(this.canvasElement, {
        shown: (stream) => this.painted(stream),
        lost: () => this.link.resync(),
        // A picture waits for its orientation's frame, so both show together.
        ready: (stream) => this.state.frames[stream.orientation] !== undefined,
      })
      this.link.attach(this.pictures)
    }
    return this.canvasElement
  }

  /** This web browser shows the pictures: the stream opens, not looked at until a view shows. */
  enable(): void {
    if (this.enabled || this.state.gone) {
      return
    }
    this.enabled = true
    this.link.start()
    this.update()
  }

  /** The page became visible or hidden. */
  visibility(visible: boolean): void {
    this.visible = visible
    this.update()
  }

  /** `view` of the device shows, or no longer does. */
  showIn(view: object, shown: boolean): void {
    if (shown) {
      this.views.add(view)
    } else {
      this.views.delete(view)
    }
    this.update()
  }

  /** Tells the Host whether the stream is looked at, when that changed. */
  private update(): void {
    const looked = this.enabled && this.visible && this.views.size > 0 && !this.state.gone
    if (looked === this.looked) {
      return
    }
    this.looked = looked
    const connected = this.link.connected
    // A stream that ended while nobody looked opens again here, and says so as it opens.
    this.link.watched(looked)
    if (connected) {
      this.send({ type: 'looked', looked })
      if (looked) {
        // A stream paused while nobody looked may have lost its connection
        // silently: its Host answers with the device's state.
        this.link.expectAnswer()
        this.seekFrames()
      }
    }
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
        this.state.device = message.device
        this.keepOrientation(message.device.orientation)
        // The device's model and orientation are known now, which its cached
        // frame is kept by; before a picture, the look is that orientation's.
        this.lookBeforePicture()
        this.seekFrames()
        break
      case 'gone':
        this.state.gone = true
        this.close()
        this.options.gone?.()
        break
      case 'stream':
        this.reported(message.orientation)
        this.link.generation({
          generation: message.generation,
          width: message.width,
          height: message.height,
          pointsWidth: message.pointsWidth,
          pointsHeight: message.pointsHeight,
          orientation: message.orientation,
        })
        // The pictures of this orientation wait for its frame.
        this.seekFrames()
        break
      case 'activity':
        this.state.activity = message.activity
        break
      case 'frame':
        if (message.frame !== null) {
          const area = { width: message.width, height: message.height, scale: message.scale }
          const frame = { orientation: message.orientation, pointsWidth: message.pointsWidth, pointsHeight: message.pointsHeight, frame: message.frame }
          // The Host answered the view's ask with the frame of this
          // orientation: another one the view needs is asked for anew.
          if (this.asked && sameArea(this.asked.area, area)) {
            this.asked = { area, orientation: message.orientation }
          }
          // A frame drawn for an area the view has left still fits until the one for its area comes.
          const held = this.state.frames[message.orientation]
          if (sameArea(area, this.area) || !held || !sameArea(held.area, this.area)) {
            this.ready({ ...frame, area }, 'host')
          }
        }
        break
      case 'screen':
        this.state.screenOff = !message.on
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

  /**
   * Decodes `frame`'s images, then makes it the frame of its orientation,
   * unless a newer one came meanwhile; a frame from the Host is kept in the
   * disk cache once it showed it can be. A frame that cannot be shown is
   * asked of the Host when it came from the cache, and is the view's failure
   * when it came from the Host.
   */
  private ready(frame: DeviceFrame, from: 'host' | 'cache'): void {
    const { orientation } = frame
    this.readying.set(orientation, frame.frame)
    const failed = (error: unknown) => {
      if (this.readying.get(orientation) !== frame.frame || this.state.gone) {
        return
      }
      this.readying.delete(orientation)
      if (from === 'cache') {
        this.askFrame(orientation)
        return
      }
      this.state.frameFailed = true
      this.options.defect('The device’s frame could not be shown', error)
    }
    if (!fitsItsScreen(frame)) {
      failed(new Error(`a ${orientation} frame whose screen has the other orientation’s shape`))
      return
    }
    decodeImages(frame.frame, this.options.decodeImage ?? decodeImage).then(() => {
      if (this.readying.get(orientation) !== frame.frame || this.state.gone) {
        return
      }
      this.readying.delete(orientation)
      if (from === 'host') {
        this.cacheFrame(frame.area, { orientation, pointsWidth: frame.pointsWidth, pointsHeight: frame.pointsHeight, frame: frame.frame })
      }
      this.state.frames[orientation] = frame
      const look = this.state.look
      if (look?.orientation === orientation) {
        // The look's own orientation: its frame changes, the rest stays.
        this.state.look = { ...look, frame: frame.frame }
      } else {
        this.lookBeforePicture()
      }
      // The Host drew the frame for a screen that turned since: the view
      // still needs the frame of the orientation the device has.
      const expected = this.expectedOrientation()
      if (expected && expected !== orientation && this.area && !this.readying.has(expected)) {
        this.seekFrame(expected, this.area)
      }
      // A picture held for this frame paints now.
      this.pictures?.retry()
    }, failed)
  }

  /**
   * Before any picture, the device shows in the frame of the orientation it
   * has, at once, once that frame is ready; a frame of another orientation
   * never shows as the device's.
   */
  private lookBeforePicture(): void {
    const look = this.state.look
    const expected = this.expectedOrientation()
    const frame = expected ? this.state.frames[expected] : undefined
    if (look?.picture || !frame || look?.orientation === expected) {
      return
    }
    this.state.look = { orientation: frame.orientation, pointsWidth: frame.pointsWidth, pointsHeight: frame.pointsHeight, frame: frame.frame, picture: null }
  }

  /** The view shows that its frame could not be shown, and the viewer asks for it again: the Host draws it anew. */
  retryFrame(): void {
    this.state.frameFailed = false
    this.asked = null
    const expected = this.expectedOrientation()
    this.askFrame(expected)
  }

  /** A picture of `stream` painted: the look is that picture's, in its orientation's frame, whole. */
  private painted(stream: DeviceStream): void {
    const frame = this.state.frames[stream.orientation]
    if (!frame) {
      return
    }
    this.state.look = {
      orientation: stream.orientation,
      pointsWidth: stream.pointsWidth,
      pointsHeight: stream.pointsHeight,
      frame: frame.frame,
      picture: stream,
    }
    this.keepOrientation(stream.orientation)
  }

  resync(): void {
    this.link.resync()
  }

  /**
   * The viewer pressed Rotate Left or Right: the press, until the device
   * has shown its effect, turns what the view shows a quarter at once.
   * Answers the press's identity, which its answer names; none when the
   * view shows nothing to turn yet.
   */
  turning(turn: Turn): number | null {
    const shown = this.shownOrientation()
    if (!shown) {
      return null
    }
    const id = this.nextTurn++
    const quarters = TURN_QUARTERS[turn]
    this.state.turning.push({ id, quarters, expected: turned(shown, quarters), answered: null, reported: [] })
    // The turned device shows in its new orientation's frame, which the disk cache may keep.
    this.seekFrames()
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
    // A device that answers with the orientation it shows kept it, as when
    // the app in front refuses the turn: nothing will turn.
    if (orientation === null || turn.reported.includes(orientation) || orientation === this.baseOrientation()) {
      this.state.turning = this.state.turning.filter((each) => each !== turn)
      return
    }
    turn.answered = orientation
  }

  /**
   * The device's pictures come in `orientation`, as it announced them,
   * whoever turned it: a press it answered with that orientation, or one
   * not answered yet that turns the device to it, is done, the device now
   * showing it, and so is every answered press before it, which the device
   * has turned past. A turn by someone else that ends where a press aims
   * ends the press too; the device's further turns show as it reports them.
   */
  private reported(orientation: Orientation): void {
    this.state.reported = orientation
    for (const turn of this.state.turning) {
      turn.reported.push(orientation)
    }
    const shows = (turn: DeviceTurn) => (turn.answered ?? turn.expected) === orientation
    const last = this.state.turning.findLastIndex(shows)
    const done = new Set(this.state.turning.filter((turn, at) => shows(turn) || (at < last && turn.answered !== null)))
    this.state.turning = this.state.turning.filter((turn) => !done.has(turn))
  }

  /**
   * The orientation the device is shown in: the one its pictures report on
   * this connection, else the one the view shows, turned by the viewer's
   * presses whose effect it has not reported, answered or not; none while
   * the view shows nothing.
   */
  shownOrientation(): Orientation | null {
    const base = this.baseOrientation()
    if (!base) {
      return null
    }
    return turned(base, this.state.turning.reduce((sum, turn) => sum + turn.quarters, 0))
  }

  /**
   * The orientation the device shows, but for the viewer's presses: the one
   * its pictures report on this connection, else the one of the picture the
   * view shows, else, before any picture, the one the Host says its screen
   * has, else the one the view shows.
   */
  private baseOrientation(): Orientation | null {
    const look = this.state.look
    return this.state.reported
      ?? (look?.picture ? look.orientation : null)
      ?? this.state.device?.orientation
      ?? look?.orientation
      ?? null
  }

  /** The quarter turns, clockwise, the view adds to `drawn`, the orientation of what it draws now. */
  pendingQuarters(drawn: Orientation): number {
    const shown = this.shownOrientation()
    return shown ? quartersBetween(drawn, shown) : 0
  }

  /**
   * The orientation the device's next look takes: the one it is shown in,
   * else the one its screen reports in the state, else the one the disk
   * cache kept.
   */
  private expectedOrientation(): Orientation | null {
    return this.shownOrientation() ?? this.state.device?.orientation ?? this.cachedOrientation
  }

  /**
   * The disk cache's key of a frame in `orientation`, drawn for `area`:
   * none for a device whose model the driver does not know, whose frame
   * another device of no model may not share.
   */
  private frameKey(orientation: Orientation, area: FrameArea): string | null {
    const model = this.model
    return model ? `frame:${model}:${area.width}x${area.height}@${area.scale}:${orientation}` : null
  }

  /** The disk cache's key of the device's last orientation. */
  private orientationKey(): string {
    return `orientation:${this.options.nativeId ?? this.options.device}`
  }

  /** Keeps a frame the module drew for `area`, the area of the request it answers, which the view may have left since. */
  private cacheFrame(area: FrameArea, frame: z.infer<typeof cachedFrameSchema>): void {
    const key = this.frameKey(frame.orientation, area)
    const cache = this.options.cache
    if (key === null || !cache) {
      return
    }
    cache.write(key, new TextEncoder().encode(JSON.stringify(frame))).catch((error: unknown) => {
      // A cache that cannot be written leaves the frame to come from the Host again.
      this.options.defect('The device frame could not be cached', error)
    })
  }

  /** Keeps the device's orientation, when it changed, for the next session's first look. */
  private keepOrientation(orientation: Orientation): void {
    if (orientation === this.cachedOrientation) {
      return
    }
    this.cachedOrientation = orientation
    this.options.cache?.write(this.orientationKey(), new TextEncoder().encode(orientation)).catch((error: unknown) => {
      // A cache that cannot be written leaves the orientation to the Host's state.
      this.options.defect('The device orientation could not be cached', error)
    })
  }

  /**
   * Finds the frames the view needs for the area it has: the expected
   * orientation's and, while a press turns it, the turned one's. Each comes
   * from the disk cache, or from the Host when the cache lacks it; before
   * the device's orientation is known, the cache's is read first.
   */
  private seekFrames(): void {
    const area = this.area
    if (!area) {
      return
    }
    const expected = this.expectedOrientation()
    if (!expected) {
      this.readCachedOrientation()
      return
    }
    for (const orientation of new Set([expected, this.shownOrientation() ?? expected])) {
      this.seekFrame(orientation, area)
    }
  }

  /**
   * Reads the device's orientation the disk cache kept, whose frame the
   * cache may keep too; without one, the Host draws the frame for the
   * orientation the device has, rather than the view waiting for the
   * Host's state to name it first.
   */
  private readCachedOrientation(): void {
    const cache = this.options.cache
    if (!cache) {
      this.askFrame(null)
      return
    }
    if (this.readOrientation) {
      return
    }
    this.readOrientation = true
    cache.read(this.orientationKey()).then((bytes) => {
      const kept = bytes && orientationSchema.safeParse(new TextDecoder().decode(bytes))
      if (!kept?.success) {
        this.askFrame(null)
      } else if (!this.state.device) {
        // The Host's word, when it came first, stands.
        this.cachedOrientation = kept.data
        this.seekFrames()
      }
    }, (error: unknown) => {
      this.options.defect('The device orientation could not be read from the cache', error)
      this.askFrame(null)
    })
  }

  private seekFrame(orientation: Orientation, area: FrameArea): void {
    if (sameArea(this.state.frames[orientation]?.area, area)) {
      return
    }
    const key = this.frameKey(orientation, area)
    const cache = this.options.cache
    if (key === null || !cache) {
      this.askFrame(orientation)
      return
    }
    if (this.reading.has(key)) {
      return
    }
    this.reading.add(key)
    cache.read(key).then((bytes) => {
      this.reading.delete(key)
      if (!sameArea(this.area, area) || sameArea(this.state.frames[orientation]?.area, area) || this.state.gone) {
        return
      }
      const kept = bytes && cachedFrameSchema.safeParse(JSON.parse(new TextDecoder().decode(bytes)))
      if (kept?.success && kept.data.orientation === orientation) {
        this.ready({ ...kept.data, area }, 'cache')
        return
      }
      // Not kept, or written by another build of the page: the Host draws it.
      this.askFrame(orientation)
    }, (error: unknown) => {
      this.reading.delete(key)
      this.options.defect('The device frame could not be read from the cache', error)
      this.askFrame(orientation)
    })
  }

  /**
   * Asks the Host to draw the device's frame for the view's area, which it
   * draws for the screen as it is: once per area and connection, and again
   * when the view needs another orientation's, as when the Host's last
   * frame was drawn for a screen that turned since.
   */
  private askFrame(orientation: Orientation | null): void {
    const area = this.area
    const asked = this.asked
    if (!area || !this.link.connected || this.state.frameFailed) {
      return
    }
    // An ask of no orientation is answered with the screen's, whichever it is.
    if (asked && sameArea(asked.area, area) && (asked.orientation === null || orientation === null || asked.orientation === orientation)) {
      return
    }
    this.asked = { area, orientation }
    this.send({ type: 'frame', ...area })
  }

  /** Without a byte for a second the connection is stalled, and what the viewer holds is let go. */
  tick(): void {
    this.link.tick()
  }

  /**
   * The view lays the device's frame into `width` × `height` device pixels,
   * `scale` of them per CSS pixel: its frame for that size comes from the
   * disk cache or the Host.
   */
  frame(width: number, height: number, scale: number): void {
    const area = { width: Math.max(1, Math.round(width)), height: Math.max(1, Math.round(height)), scale }
    if (sameArea(this.area, area)) {
      return
    }
    this.area = area
    this.seekFrames()
  }

  /**
   * The view shows the device's screen at `width` × `height` device pixels:
   * the Host captures it no larger (`crates/plugin-mobile/page/src/index.ts`).
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
    if (this.screenSize && this.link.connected) {
      this.send({ type: 'shown', ...this.screenSize })
    }
  }

  /** Wake: the device's screen turns on, and never off (`crates/plugin-mobile/page/src/index.ts`). */
  wake(): void {
    this.send({ type: 'wake' })
  }

  /** The viewer's touch or key; dropped while the stream is not looked at or stalls. */
  input(input: ViewerInput): void {
    if (!this.looked || this.state.connection !== 'live') {
      return
    }
    this.send({ type: 'input', input })
  }

  release(): void {
    this.send({ type: 'release' })
  }

  /** The device closed, or the store let it go: the stream ends and nothing reopens it. */
  close(): void {
    this.link.close()
    this.pictures?.stop()
  }
}
