/**
 * One live view of the conversation's browser: the page's side of the protocol. It keeps what the view shows,
 * hands pictures to a decoder, acknowledges what the viewer saw, and sends
 * the viewer's input while the stream is alive. A view whose stream goes
 * silent for as long as a page socket may is broken, and a view that ends
 * opens again after the page's reconnect waits.
 */
import { liveViewerMessageSchema, panelReasonSchema, type BrowserViewport, type CursorRegion, type LiveControl, type LiveDialog, type LiveDownload, type LiveMenu, type LiveTab, type LiveViewerMessage, type PanelReason } from '../generated/plugin'
import { LIVE_CAPTURE_FAILED, LIVE_CAPTURE_STOPPED, LIVE_CAPTURE_UNAVAILABLE, LIVE_FILE_CHUNK_BYTES } from '../generated/plugin'
import { reactive } from 'vue'
import { LiveLink, type LiveConnection, type LiveVideoFrame, type PictureSink as LivePictureSink } from '@demicodes/plugin-sdk'
import type { OpenUserStream } from '@demicodes/plugin-sdk'
import { LiveFrameReader, encodeFile, encodeMessage, type LiveFrame } from './frames'
import type { PanelSize } from './view'

/**
 * A stream generation as the module names it: its pictures' size in pixels,
 * the viewport they show and their scale of its device pixels.
 */
export interface LiveStream {
  tab: string
  generation: number
  width: number
  height: number
  viewport: BrowserViewport
  scale: number
}

/** What shows the pictures: a decoder in the page, or the gallery's canvas. */
export type PictureSink = LivePictureSink<LiveStream>

export { REFUSED_FRAME, SILENT_STREAM, type LiveConnection } from '@demicodes/plugin-sdk'

/**
 * The reasons of the notices that leave the watched tab without a picture,
 * which the view shows in place of the picture; every other notice the
 * picture still shows through. Each is a
 * reason of the plugin's set, which the page has words for.
 */
export type PicturelessReason = typeof LIVE_CAPTURE_UNAVAILABLE | typeof LIVE_CAPTURE_FAILED | typeof LIVE_CAPTURE_STOPPED

const PICTURELESS: readonly PicturelessReason[] = [LIVE_CAPTURE_UNAVAILABLE, LIVE_CAPTURE_FAILED, LIVE_CAPTURE_STOPPED]

function isPictureless(reason: PanelReason): reason is PicturelessReason {
  return PICTURELESS.some((pictureless) => pictureless === reason)
}

export interface LiveState {
  connection: LiveConnection
  /** Whether the conversation's browser runs; a view can offer to start one. */
  running: boolean
  tabs: LiveTab[]
  watched: string | null
  dialog: { tab: string; dialog: LiveDialog } | null
  /** The browser's menu the viewer's right click asked for, until it is chosen from or dismissed. */
  menu: { tab: string; menu: LiveMenu } | null
  controls: LiveControl[]
  /** Where on the watched tab each cursor applies, in tab CSS pixels; a later region over an earlier one wins. */
  regions: CursorRegion[]
  /** The cursor the watched tab's observer resolved at the pointer, where no region decides. */
  cursor: { cursor: string; editable: boolean }
  /**
   * Why the watched tab shows no picture: the code of a notice that leaves
   * none, until a picture of the tab or another watched tab ends it.
   */
  pictureless: PicturelessReason | null
  /** Why the view ended, once it did. */
  ended: string | null
}

/** The panel the viewer shows the watched tab in, and its screen, as the page last measured them. */
export interface PanelReport {
  panel: PanelSize
  devicePixelRatio: number
  screen: PanelSize
}

export interface LiveSessionOptions {
  open: OpenUserStream
  /** The panel the view sizes the watched tab by, which the page keeps; none until it measured one. */
  panel: () => PanelReport | null
  platform: 'mac' | 'windows' | 'linux' | 'other'
  /** Text the watched tab copied, for the viewer's own clipboard. */
  onClipboard?: (text: string) => void
  /**
   * The code of a notice the picture still shows through, once per notice:
   * something the module could not do for the viewer. Its message is the
   * Host's, which the Host's log keeps.
   */
  onNotice?: (reason: PanelReason) => void
  /** The conversation browser's tabs, each time the view reports them, with the list's number in the Host's sequence. */
  onTabs?: (tabs: LiveTab[], list: number) => void
  /**
   * The view ended; it opens again by itself, since a Host that becomes
   * reachable again, or a conversation browser started later, reaches the
   * page through a new view.
   */
  onEnded?: (reason: string) => void
  /** The page showed a picture of the browser tab `tab`. */
  onPicture?: (tab: string) => void
  /** The downloads the user started in the browser tab `tab`, the newest last, each time they change. */
  onDownloads?: (tab: string, downloads: LiveDownload[]) => void
  /** Reports a defect of the page, such as a message the protocol refuses. */
  defect: (message: string, error: unknown) => void
  now?: () => number
}

/** An upload the viewer started, whose bytes follow. */
interface PendingUpload {
  files: readonly File[]
  upload: number
}

export class LiveSession {
  /** What the view shows; the components follow it. */
  readonly state: LiveState = reactive({
    connection: 'opening',
    running: false,
    tabs: [],
    watched: null,
    dialog: null,
    menu: null,
    controls: [],
    regions: [],
    cursor: { cursor: 'default', editable: false },
    pictureless: null,
    ended: null,
  })

  private readonly link: LiveLink<LiveFrame, LiveStream>
  private uploads = 0
  /** The viewer's last press, a button or a key, in the tab it watched, and when. */
  private pressed: { tab: string; at: number } | null = null

  constructor(private readonly options: LiveSessionOptions) {
    this.link = new LiveLink(this.state, {
      open: options.open,
      frames: () => new LiveFrameReader(),
      // The panel comes before the tab, so the module sizes the tab before it
      // captures it; a view that opens again takes
      // up where the page left off.
      opened: () => {
        this.send({ type: 'hello', platform: options.platform })
        this.panel()
        if (this.state.watched) {
          this.send({ type: 'watch', tab: this.state.watched })
        }
      },
      received: (frame) => this.receive(frame),
      ended: (reason) => {
        this.state.dialog = null
        this.state.menu = null
        this.state.pictureless = null
        this.state.controls = []
        this.state.regions = []
        this.state.running = false
        this.state.tabs = []
        options.onEnded?.(reason)
      },
      stalled: () => this.release(),
      watching: (stream) => stream.tab === this.state.watched,
      keyframe: (stream) => this.send({ type: 'keyframe', generation: stream.generation }),
      defect: options.defect,
      now: options.now,
    })
  }

  /** What shows this view's pictures, once the view has a canvas. */
  attach(pictures: PictureSink): void {
    this.link.attach(pictures)
  }

  private time(): number {
    return this.options.now ? this.options.now() : performance.now()
  }

  start(): void {
    this.link.start()
  }

  /**
   * A view waiting to reconnect connects at once, its waits started over:
   * what it waited for, such as a browser that did not run yet, may be there
   * now.
   */
  reconnect(): void {
    this.link.reconnect()
  }

  /** The page is done with the view: nothing reopens it, and it has nothing to tell. */
  close(): void {
    this.link.close()
  }

  /**
   * The module ends a view that sends it a message the protocol refuses, so a
   * message that does not fit is this page's defect: it is reported and never
   * sent, and the view stays.
   */
  private send(message: LiveViewerMessage): void {
    const checked = liveViewerMessageSchema.safeParse(message)
    if (!checked.success) {
      this.options.defect(`The live view built an invalid ${message.type} message`, checked.error)
      return
    }
    this.link.send(encodeMessage(checked.data))
  }

  /** Input the page discards rather than queueing while the stream stalls. */
  private operate(message: LiveViewerMessage): void {
    if (this.state.connection !== 'live') {
      return
    }
    this.send(message)
  }

  private receive(frame: LiveFrame): void {
    if (frame.kind === 'video') {
      if (this.link.show(frame.frame)) {
        // A picture of the watched tab is the capture working again.
        this.state.pictureless = null
      }
      return
    }
    const message = frame.message
    switch (message.type) {
      case 'state':
        this.link.live()
        this.state.running = message.running
        this.state.tabs = message.tabs
        this.options.onTabs?.(message.tabs, message.list)
        // The page decides what it watches. The module says otherwise when
        // the tab went away, or when its answer crossed a newer wish on the
        // way: a tab that is still there is asked for again.
        if (this.state.watched !== message.watched) {
          this.state.controls = []
          this.state.regions = []
          const wanted = this.state.watched
          if (wanted !== null && message.tabs.some((tab) => tab.id === wanted)) {
            this.send({ type: 'watch', tab: wanted })
          } else {
            this.state.watched = message.watched
          }
        }
        break
      case 'stream':
        // Video generations change independently of the watched document's controls.
        this.link.generation({
          tab: message.tab,
          generation: message.generation,
          width: message.width,
          height: message.height,
          viewport: message.viewport,
          scale: message.scale,
        })
        break
      case 'heartbeat':
        break
      case 'cursors':
        this.state.regions = message.tab === this.state.watched ? message.regions : []
        break
      case 'cursor':
        this.state.cursor = { cursor: message.cursor, editable: message.editable }
        break
      case 'controls':
        this.state.controls = message.tab === this.state.watched ? message.controls : []
        break
      case 'clipboard':
        this.options.onClipboard?.(message.text)
        break
      case 'dialog':
        this.state.dialog = message.dialog ? { tab: message.tab, dialog: message.dialog } : null
        break
      case 'menu':
        this.state.menu = message.tab === this.state.watched ? { tab: message.tab, menu: message.menu } : null
        break
      case 'downloads':
        this.options.onDownloads?.(message.tab, message.downloads)
        break
      case 'choice':
        break
      case 'notice': {
        const reason = panelReasonSchema.safeParse(message.code)
        if (!reason.success) {
          // The module ships with this page, so a notice of a reason the page has no words for is its defect.
          this.options.defect('The live view sent a notice of a reason the page does not know', reason.error)
        } else if (isPictureless(reason.data)) {
          this.state.pictureless = reason.data
        } else {
          this.options.onNotice?.(reason.data)
        }
        break
      }
      case 'ended':
        this.link.end(message.reason)
        break
    }
  }

  /** The page painted a picture of `stream`'s tab. */
  showed(stream: LiveStream): void {
    this.options.onPicture?.(stream.tab)
  }

  /** The decoder lost the stream: the next frame must be a key frame. */
  resync(): void {
    this.link.resync()
  }

  /**
   * Time passes: without a byte for a second the connection is stalled, and
   * input is discarded rather than delivered all at once later.
   */
  tick(): void {
    this.link.tick()
  }

  /** Tells the module the page's panel, as the page last measured it. */
  panel(): void {
    const report = this.options.panel()
    if (!report) {
      return
    }
    this.send({
      type: 'panel',
      width: report.panel.width,
      height: report.panel.height,
      devicePixelRatio: Math.max(0.5, Math.min(4, report.devicePixelRatio)),
      screenWidth: report.screen.width,
      screenHeight: report.screen.height,
    })
  }

  watch(tab: string | null): void {
    if (tab === this.state.watched) {
      return
    }
    this.state.watched = tab
    this.state.controls = []
    this.state.regions = []
    this.state.dialog = null
    this.state.menu = null
    // The module says again whether it can capture the tab now watched.
    this.state.pictureless = null
    this.link.unwatched()
    this.send({ type: 'watch', tab })
  }

  /**
   * Asks the Host to capture again after it stopped trying: it recreates its
   * capture extension, and the view says it tries again until a picture or
   * another notice.
   */
  recapture(): void {
    if (this.state.pictureless !== LIVE_CAPTURE_STOPPED) {
      return
    }
    this.state.pictureless = LIVE_CAPTURE_FAILED
    this.send({ type: 'recapture' })
  }

  mode(tab: string, mode: 'web' | 'mobile'): void {
    this.send({ type: 'mode', tab, mode })
  }

  answerDialog(accept: boolean, text?: string): void {
    const dialog = this.state.dialog
    if (!dialog) {
      return
    }
    this.state.dialog = null
    this.send({ type: 'dialog', tab: dialog.tab, accept, ...(text === undefined ? {} : { text }) })
  }

  /** The browser's menu was chosen from or dismissed. */
  closeMenu(): void {
    this.state.menu = null
  }

  /** Pointer movement is not an operation; everything else is. A press is the viewer's own click or key. */
  input(message: LiveViewerMessage): void {
    const input = message.type === 'input' ? message.input : null
    const moving = input?.type === 'pointer' && input.action === 'move'
    if (moving) {
      if (this.state.connection === 'live') {
        this.send(message)
      }
      return
    }
    if (this.state.connection === 'live' && message.type === 'input' && (input?.type === 'pointer' || input?.type === 'key') && input.action === 'down') {
      this.pressed = { tab: message.tab, at: this.time() }
    }
    this.operate(message)
  }

  /** Whether the viewer pressed a button or a key in `tab` within the last `within` milliseconds. */
  pressedLately(tab: string, within: number): boolean {
    return this.pressed !== null && this.pressed.tab === tab && this.time() - this.pressed.at <= within
  }

  /** The viewer's choice in a native control, for the revision it saw. */
  choose(control: LiveControl, value: string, indices: readonly number[]): void {
    const tab = this.state.watched
    if (!tab) {
      return
    }
    this.operate({
      type: 'choice',
      tab,
      token: control.token,
      revision: control.revision,
      value,
      indices: [...indices],
    })
  }

  /** Files the viewer chose; their bytes follow this message. */
  async upload(control: LiveControl, files: readonly File[]): Promise<void> {
    const tab = this.state.watched
    if (!tab || this.state.connection !== 'live') {
      return
    }
    const pending: PendingUpload = { files, upload: this.uploads++ }
    this.operate({
      type: 'upload',
      tab,
      token: control.token,
      revision: control.revision,
      upload: pending.upload,
      files: files.map((file) => ({ name: file.name, mimeType: file.type, size: file.size })),
    })
    for (const [index, file] of files.entries()) {
      const reader = file.stream().getReader()
      for (;;) {
        const { value, done } = await reader.read()
        if (done || !this.link.connected) {
          break
        }
        for (let start = 0; start < value.length; start += LIVE_FILE_CHUNK_BYTES) {
          this.link.send(encodeFile(pending.upload, index, value.subarray(start, start + LIVE_FILE_CHUNK_BYTES)))
        }
      }
    }
  }

  /** Release every key and button this view holds. */
  release(): void {
    this.send({ type: 'release' })
  }
}
