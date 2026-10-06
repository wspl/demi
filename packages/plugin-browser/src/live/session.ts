/**
 * One live view of the conversation's browser (`live-view.md` § The
 * stream): the page's side of the protocol. It keeps what the view shows,
 * hands pictures to a decoder, acknowledges what the viewer saw, and sends
 * the viewer's input while the stream is alive. A view whose stream goes
 * silent for as long as a page socket may is broken, and a view that ends
 * opens again after the page's reconnect waits (`web-application.md`
 * § Liveness and reconnection).
 */
import { liveViewerMessageSchema, type BrowserViewport, type CursorRegion, type LiveControl, type LiveDialog, type LiveTab, type LiveViewerMessage } from '../generated/plugin'
import { LIVE_CAPTURE_FAILED, LIVE_FILE_CHUNK_BYTES, LIVE_STALL_MS } from '../generated/plugin'
import { reactive } from 'vue'
import { waitToReconnect, watchSilence, type ReconnectWait, type SilenceWatch } from '@demicodes/plugin-sdk'
import type { OpenUserStream, StreamBytes, UserStream } from '@demicodes/plugin-sdk'
import { LiveFrameReader, encodeFile, encodeMessage, type LiveFrame, type LiveVideoFrame } from './frames'
import type { PanelSize } from './view'

/**
 * A stream generation as the module names it: its pictures' size in pixels,
 * the viewport they show and their scale of its device pixels
 * (`live-view.md` § Modes).
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
export interface PictureSink {
  /** Frames of this generation follow. */
  start(stream: LiveStream): void
  show(frame: LiveVideoFrame): void
  /** Nothing is watched any more. */
  stop(): void
}

/** Why the page ended a view whose module sent a frame the protocol refuses. */
export const REFUSED_FRAME = 'invalid_frame'

/** Why the page ended a view whose stream brought nothing, heartbeats included, for as long as a page socket may. */
export const SILENT_STREAM = 'silent'

export type LiveConnection = 'opening' | 'live' | 'stalled' | 'ended'

export interface LiveState {
  connection: LiveConnection
  /** Whether the conversation's browser runs; a view can offer to start one. */
  running: boolean
  tabs: LiveTab[]
  watched: string | null
  dialog: { tab: string; dialog: LiveDialog } | null
  controls: LiveControl[]
  /** Where on the watched tab each cursor applies, in tab CSS pixels; a later region over an earlier one wins. */
  regions: CursorRegion[]
  /** The cursor the watched tab's observer resolved at the pointer, where no region decides. */
  cursor: { cursor: string; editable: boolean }
  /** The latest thing that failed, for the view to show. */
  notice: { code: string; message: string } | null
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
  /** The conversation browser's tabs, each time the view reports them. */
  onTabs?: (tabs: LiveTab[]) => void
  /**
   * The view ended; it opens again by itself, since a Host that becomes
   * reachable again, or a conversation browser started later, reaches the
   * page through a new view.
   */
  onEnded?: (reason: string) => void
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
    controls: [],
    regions: [],
    cursor: { cursor: 'default', editable: false },
    notice: null,
    ended: null,
  })

  private stream: UserStream | null = null
  /** The watch over the stream's silence, while there is a stream. */
  private silence: SilenceWatch | null = null
  private pictures: PictureSink | null = null
  /** The pictures the stream sends now; none before its first. */
  private video: LiveStream | null = null
  private uploads = 0
  private received: number
  /** When the page last asked for a key frame; never, at first. */
  private resynced = Number.NEGATIVE_INFINITY
  /** Views that ended in a row since one last worked. */
  private failures = 0
  private reopening: ReconnectWait | null = null

  constructor(private readonly options: LiveSessionOptions) {
    this.received = this.time()
  }

  /** What shows this view's pictures, once the view has a canvas. */
  attach(pictures: PictureSink): void {
    this.pictures?.stop()
    this.pictures = pictures
    // A canvas that comes after its stream started missed the generation's key frame, and a still page
    // sends no other: it starts on the generation, from a key frame it asks for. Pictures of a tab the
    // view no longer watches are not this canvas's.
    if (this.video && this.video.tab === this.state.watched) {
      pictures.start(this.video)
      this.resync()
    }
  }

  private time(): number {
    return this.options.now ? this.options.now() : performance.now()
  }

  start(): void {
    this.received = this.time()
    // Each view reads its own stream from the first byte.
    const reader = new LiveFrameReader()
    // A view ends once: the module's `ended` and the socket's close both say so, and a
    // stream this session already left says nothing about the one that followed it.
    const stream = this.options.open({
      data: (bytes) => {
        if (this.stream === stream) {
          this.receive(reader, bytes)
        }
      },
      closed: (reason) => {
        if (this.stream === stream) {
          this.end(reason)
        }
      },
    })
    this.stream = stream
    // The module speaks at least every quarter second, so the page's rule
    // for a silent socket holds for the view as for the page's other sockets.
    this.silence = watchSilence(() => this.end(SILENT_STREAM))
    this.send({ type: 'hello', platform: this.options.platform })
    // The panel comes before the tab, so the module sizes the tab before it
    // captures it (`live-view.md` § Delivery); a view that opens again takes
    // up where the page left off.
    this.panel()
    if (this.state.watched) {
      this.send({ type: 'watch', tab: this.state.watched })
    }
  }

  /**
   * A view waiting to reconnect connects at once, its waits started over:
   * what it waited for, such as a browser that did not run yet, may be there
   * now (`live-view.md` § A browser tab in the panel).
   */
  reconnect(): void {
    if (this.reopening === null) {
      return
    }
    this.reopening.cancel()
    this.reopening = null
    this.failures = 0
    this.start()
  }

  /** The page is done with the view: nothing reopens it, and it has nothing to tell. */
  close(): void {
    this.reopening?.cancel()
    this.reopening = null
    this.dropStream()
    this.state.connection = 'ended'
  }

  /** Lets the stream go, with its watch and its pictures: nothing it says afterwards reaches the view. */
  private dropStream(): void {
    this.silence?.stop()
    this.silence = null
    this.pictures?.stop()
    this.video = null
    const stream = this.stream
    this.stream = null
    stream?.close()
  }

  private end(reason: string): void {
    this.dropStream()
    this.state.ended = reason
    this.state.dialog = null
    this.state.controls = []
    this.state.regions = []
    this.options.onEnded?.(reason)
    this.failures += 1
    this.state.connection = 'opening'
    this.state.running = false
    this.state.tabs = []
    this.reopening = waitToReconnect(this.failures, () => {
      this.reopening = null
      this.start()
    })
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
    this.stream?.send(encodeMessage(checked.data))
  }

  /** Input the page discards rather than queueing while the stream stalls. */
  private operate(message: LiveViewerMessage): void {
    if (this.state.connection !== 'live') {
      return
    }
    this.send(message)
  }

  private receive(reader: LiveFrameReader, bytes: Uint8Array): void {
    this.received = this.time()
    this.silence?.heard()
    if (this.state.connection === 'stalled') {
      this.state.connection = 'live'
      // What the decoder missed while nothing arrived starts again.
      this.resync()
    }
    let frames: LiveFrame[]
    try {
      frames = reader.read(bytes)
    } catch (error) {
      // The module ships with this page, so a frame the protocol refuses is its defect.
      this.options.defect('The live view received a frame the protocol refuses', error)
      this.end(REFUSED_FRAME)
      return
    }
    for (const frame of frames) {
      if (frame.kind === 'video') {
        this.picture(frame.frame)
        continue
      }
      const message = frame.message
      switch (message.type) {
        case 'state':
          this.failures = 0
          this.state.ended = null
          this.state.connection = this.state.connection === 'opening' ? 'live' : this.state.connection
          this.state.running = message.running
          this.state.tabs = message.tabs
          this.options.onTabs?.(message.tabs)
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
          this.video = {
            tab: message.tab,
            generation: message.generation,
            width: message.width,
            height: message.height,
            viewport: message.viewport,
            scale: message.scale,
          }
          // Video generations change independently of the watched document's controls.
          this.pictures?.start(this.video)
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
        case 'choice':
          break
        case 'notice':
          this.state.notice = { code: message.code, message: message.message }
          break
        case 'ended':
          this.end(message.reason)
          break
      }
    }
  }

  private picture(frame: LiveVideoFrame): void {
    if (frame.generation !== this.video?.generation) {
      return
    }
    this.pictures?.show(frame)
    // A picture of the watched tab is the view working again: what it could not do before no longer holds.
    if (this.state.notice?.code === LIVE_CAPTURE_FAILED) {
      this.state.notice = null
    }
  }

  /** The page showed a frame; the module paces itself by these. */
  showed(generation: number, sequence: number, decodeQueue: number): void {
    if (generation === this.video?.generation) {
      this.send({ type: 'ack', generation, sequence, decodeQueue })
    }
  }

  /** The decoder lost the stream: the next frame must be a key frame. */
  resync(): void {
    const now = this.time()
    if (!this.video || now - this.resynced < 1000) {
      return
    }
    this.resynced = now
    this.send({ type: 'keyframe', generation: this.video.generation })
  }

  /**
   * Time passes: without a byte for a second the connection is stalled, and
   * input is discarded rather than delivered all at once later.
   */
  tick(): void {
    if (this.state.connection === 'live' && this.time() - this.received > LIVE_STALL_MS) {
      this.state.connection = 'stalled'
      this.release()
    }
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
    this.pictures?.stop()
    this.send({ type: 'watch', tab })
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

  /** Pointer movement is not an operation; everything else is. */
  input(message: LiveViewerMessage): void {
    const moving = message.type === 'pointer' && message.action === 'move'
    if (moving) {
      if (this.state.connection === 'live') {
        this.send(message)
      }
      return
    }
    this.operate(message)
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
        if (done || !this.stream) {
          break
        }
        for (let start = 0; start < value.length; start += LIVE_FILE_CHUNK_BYTES) {
          this.stream.send(encodeFile(pending.upload, index, value.subarray(start, start + LIVE_FILE_CHUNK_BYTES)))
        }
      }
    }
  }

  /** Release every key and button this view holds. */
  release(): void {
    this.send({ type: 'release' })
  }
}
