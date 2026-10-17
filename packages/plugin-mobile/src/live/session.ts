/**
 * One live view of the conversation's simulators (`mobile.md` § The live
 * view): the page's side of the `mobile` stream, over the live view's link
 * that the browser's view uses too. It keeps what the view shows, sends the
 * viewer's touches and keys, follows the agent's activity and the frame of
 * the watched device, and watches a device only while one of its tabs is
 * shown in a visible page, so a device nobody looks at gets its hour.
 */
import { reactive } from 'vue'
import {
  LiveFrameReader,
  LiveLink,
  encodeControl,
  type LiveActivity,
  type LiveConnection,
  type LiveRawFrame,
  type OpenUserStream,
  type PictureSink as LivePictureSink,
  type ViewerInput,
} from '@demicodes/plugin-sdk'
import {
  LIVE_CAPTURE_FAILED,
  LIVE_CAPTURE_UNAVAILABLE,
  liveModuleMessageSchema,
  liveViewerMessageSchema,
  panelReasonSchema,
  type LiveDevice,
  type LiveFrame,
  type LiveModuleMessage,
  type LiveViewerMessage,
  type MobilePlatform,
  type Orientation,
  type PanelReason,
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

/** The watched device's frame, for the orientation it was drawn for. */
export interface DeviceFrame {
  device: string
  orientation: Orientation
  frame: LiveFrame
}

export interface LiveState {
  connection: LiveConnection
  /** The conversation's open devices on this Host, each with whether it records. */
  devices: LiveDevice[]
  watched: string | null
  /** The agent's activity on the watched device. */
  activity: LiveActivity
  /** The watched device's frame, once the module drew it for the view's size. */
  frame: DeviceFrame | null
  pictureless: PicturelessReason | null
  ended: string | null
}

export interface MobileSessionOptions {
  open: OpenUserStream
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
    frame: null,
    pictureless: null,
    ended: null,
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
  private area: { width: number; height: number; scale: number } | null = null

  constructor(private readonly options: MobileSessionOptions) {
    this.link = new LiveLink(this.state, {
      open: options.open,
      frames: () => new DeviceFrames(),
      opened: () => {
        this.send({ type: 'hello', platform: options.platform })
        if (this.state.watched) {
          this.send({ type: 'watch', device: this.state.watched })
          this.askFrame()
        }
      },
      received: (frame) => this.receive(frame),
      ended: () => {
        this.state.pictureless = null
        this.state.activity = IDLE
        this.state.frame = null
        this.state.devices = []
      },
      stalled: () => this.release(),
      watching: (stream) => stream.device === this.state.watched,
      acknowledge: (stream, sequence, decodeQueue) =>
        this.send({ type: 'ack', generation: stream.generation, sequence, decodeQueue }),
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
        this.link.generation({
          device: message.device,
          generation: message.generation,
          width: message.width,
          height: message.height,
          pointsWidth: message.pointsWidth,
          pointsHeight: message.pointsHeight,
          orientation: message.orientation,
        })
        break
      case 'activity':
        if (message.device === this.state.watched) {
          this.state.activity = message.activity
        }
        break
      case 'frame':
        if (message.device === this.state.watched) {
          this.state.frame = { device: message.device, orientation: message.orientation, frame: message.frame }
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

  showed(generation: number, sequence: number, decodeQueue: number): void {
    this.link.showed(generation, sequence, decodeQueue)
  }

  resync(): void {
    this.link.resync()
  }

  /** Without a byte for a second the connection is stalled, and what the viewer holds is let go. */
  tick(): void {
    this.link.tick()
  }

  /**
   * This web browser shows the pictures: the stream opens once a device
   * tab is shown, as the browser's view does (`live-view.md` § One view per
   * page), and at once when one already is.
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
    this.state.frame = null
    this.link.unwatched()
    if (!this.link.active) {
      if (device !== null && this.enabled) {
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
      this.send({ type: 'frame', device: this.state.watched, width: this.area.width, height: this.area.height, scale: this.area.scale })
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
