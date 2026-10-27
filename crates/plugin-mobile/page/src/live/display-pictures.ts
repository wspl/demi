/**
 * One display's pictures of a hinged foldable (`./HingedDevice.vue`), on a
 * stream of their own: its 3D view shows each display on its own face, lit
 * or not, as Xcode's Device Hub does, so the display the guest lights shows
 * the moment the guest draws on it.
 *
 * The stream carries the pictures alone: the device's own stream
 * (`./session.ts`) carries its state, its frame and the viewer's input. It
 * is looked at while a view of the device shows, sized to the face that
 * shows it, and it opens again as the device's own does.
 */
import { reactive } from 'vue'
import { CanvasPictures, LiveLink, encodeControl, type LiveConnection, type OpenUserStream } from '@demicodes/plugin-sdk'
import {
  LIVE_CAPTURE_FAILED,
  LIVE_CAPTURE_UNAVAILABLE,
  liveViewerMessageSchema,
  type FoldDisplay,
  type LiveViewerMessage,
} from '../generated/plugin'
import { DeviceFrames, type DeviceStream, type DeviceWait, type Frame, type PicturelessReason } from './session'

export interface DisplayPicturesOptions {
  open: OpenUserStream
  platform: 'mac' | 'windows' | 'linux' | 'other'
  device: string
  display: FoldDisplay
  defect(message: string, error: unknown): void
  now?(): number
}

export interface DisplayPicturesState {
  connection: LiveConnection
  ended: string | null
  /** The pictures the canvas shows, once the first is painted. */
  look: { picture: DeviceStream } | null
  pictureless: PicturelessReason | null
  /** A display's stream has no frame, so none fails. */
  frameFailed: false
}

export class DisplayPictures {
  readonly state: DisplayPicturesState = reactive({
    connection: 'opening',
    ended: null,
    look: null,
    pictureless: null,
    frameFailed: false,
  })
  private readonly link: LiveLink<Frame, DeviceStream>
  private enabled = false
  private visible = true
  private readonly views = new Set<object>()
  private looked = false
  /** The device pixels the face shows the display at, as it last measured them. */
  private size: { width: number; height: number } | null = null
  private canvasElement: HTMLCanvasElement | null = null
  private pictures: CanvasPictures<DeviceStream> | null = null

  constructor(private readonly options: DisplayPicturesOptions) {
    this.link = new LiveLink(this.state, {
      view: 'mobile',
      open: options.open,
      frames: () => new DeviceFrames(),
      opened: () => {
        this.send({ type: 'hello', platform: options.platform, device: options.device, display: options.display })
        if (this.looked) {
          this.send({ type: 'looked', looked: true })
          this.link.expectAnswer()
          this.tellSize()
        }
      },
      received: (frame) => this.receive(frame),
      ended: () => {
        this.state.pictureless = null
      },
      // A display's stream holds nothing of the viewer's.
      stalled: () => {},
      watching: () => true,
      keyframe: (stream) => this.send({ type: 'keyframe', generation: stream.generation }),
      defect: options.defect,
      now: options.now,
    })
    this.link.watched(false)
  }

  /** The canvas its pictures paint on, made with its decoder when a view first asks. */
  get canvas(): HTMLCanvasElement {
    if (!this.canvasElement) {
      this.canvasElement = document.createElement('canvas')
      this.pictures = new CanvasPictures<DeviceStream>(this.canvasElement, {
        shown: (stream) => {
          this.link.drawn()
          this.state.look = { picture: stream }
        },
        lost: () => this.link.resync(),
        ready: () => true,
      })
      this.link.attach(this.pictures)
    }
    return this.canvasElement
  }

  /** What a view of it waits for before its first picture; none once it shows one. */
  get waiting(): DeviceWait {
    if (this.state.look || this.state.pictureless) {
      return null
    }
    if (this.state.connection !== 'live') {
      return this.reconnecting ? 'reconnecting' : 'connecting'
    }
    return 'screen'
  }

  /** Whether the stream ended and opens again, its last picture still showing. */
  get reconnecting(): boolean {
    return this.state.connection === 'opening' && this.state.ended !== null
  }

  /** A display's stream has no frame to ask for again. */
  retryFrame(): void {}

  enable(): void {
    if (this.enabled) {
      return
    }
    this.enabled = true
    this.link.start()
    this.update()
  }

  visibility(visible: boolean): void {
    this.visible = visible
    this.update()
  }

  /** `view` of the device shows the display, or no longer does. */
  showIn(view: object, shown: boolean): void {
    if (shown) {
      this.views.add(view)
    } else {
      this.views.delete(view)
    }
    this.update()
  }

  /** The face shows the display at `width` × `height` device pixels, which its pictures need no more than. */
  showsAt(width: number, height: number): void {
    const size = { width: Math.max(1, Math.round(width)), height: Math.max(1, Math.round(height)) }
    if (this.size && this.size.width === size.width && this.size.height === size.height) {
      return
    }
    this.size = size
    this.tellSize()
  }

  tick(): void {
    this.link.tick()
  }

  close(): void {
    this.link.close()
    this.pictures?.stop()
  }

  private update(): void {
    const looked = this.enabled && this.visible && this.views.size > 0
    if (looked === this.looked) {
      return
    }
    this.looked = looked
    const connected = this.link.connected
    this.link.watched(looked)
    if (connected) {
      this.send({ type: 'looked', looked })
      if (looked) {
        this.link.expectAnswer()
        this.tellSize()
      }
    }
  }

  private tellSize(): void {
    if (this.size && this.link.connected) {
      this.send({ type: 'shown', ...this.size })
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
        break
      case 'stream':
        this.link.generation({
          generation: message.generation,
          width: message.width,
          height: message.height,
          pointsWidth: message.pointsWidth,
          pointsHeight: message.pointsHeight,
          orientation: message.orientation,
        })
        break
      case 'notice':
        if (message.code === LIVE_CAPTURE_FAILED || message.code === LIVE_CAPTURE_UNAVAILABLE) {
          this.state.pictureless = message.code
        }
        break
      // The device's own stream says the rest.
      default:
        break
    }
  }
}
