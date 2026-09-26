/**
 * One live view of the conversation's browser (`live-view.md` § The
 * stream): the page's side of the protocol. It keeps what the view shows,
 * hands pictures to a decoder, acknowledges what the viewer saw, and sends
 * the viewer's input while the stream is alive.
 */
import {
  LIVE_CAPTURE_FAILED,
  LIVE_FILE_CHUNK_BYTES,
  LIVE_STALL_MS,
  liveViewerMessageSchema,
  type BrowserViewport,
  type LiveControl,
  type LiveDialog,
  type LiveTab,
  type LiveViewerMessage,
} from '@demicodes/protocol'
import { reactive } from 'vue'
import { reportError } from '../infra/errors'
import { LiveFrameReader, encodeFile, encodeMessage, type LiveBytes, type LiveFrame, type LiveVideoFrame } from './frames'
import type { PanelSize } from './view'

/** The bytes of one view, as the product or the gallery carries them. */
export interface LiveStream {
  send(bytes: LiveBytes): void
  close(): void
}

export interface LiveStreamHandlers {
  data(bytes: Uint8Array): void
  /** The stream ended; `reason` is the close reason the backend gave. */
  closed(reason: string): void
}

export type OpenLiveStream = (handlers: LiveStreamHandlers) => LiveStream

/** What shows the pictures: a decoder in the page, or the gallery's canvas. */
export interface PictureSink {
  /** Frames of this generation follow, at this size in device pixels. */
  start(generation: number, width: number, height: number): void
  show(frame: LiveVideoFrame): void
  /** Nothing is watched any more. */
  stop(): void
}

/** Why the page ended a view whose module sent a frame the protocol refuses. */
export const REFUSED_FRAME = 'invalid_frame'

export type LiveConnection = 'opening' | 'live' | 'stalled' | 'ended'

export interface LiveState {
  connection: LiveConnection
  /** Whether the conversation's browser runs; a view can offer to start one. */
  running: boolean
  tabs: LiveTab[]
  watched: string | null
  dialog: { tab: string; dialog: LiveDialog } | null
  controls: LiveControl[]
  cursor: { cursor: string; editable: boolean }
  /** The latest thing that failed, for the view to show. */
  notice: { code: string; message: string } | null
  /** Why the view ended, once it did. */
  ended: string | null
}

export interface LiveSessionOptions {
  open: OpenLiveStream
  platform: 'mac' | 'windows' | 'linux' | 'other'
  /** Text the watched tab copied, for the viewer's own clipboard. */
  onClipboard?: (text: string) => void
  /** The browser's tabs, each time the view reports them. */
  onTabs?: (tabs: LiveTab[]) => void
  /** The view ended; it may open again by itself. */
  onEnded?: (reason: string) => void
  /**
   * How long to wait before opening the view again after the stream ended,
   * or null to leave it ended. A Host that becomes reachable again, or a
   * browser started later, reaches the page through a new view.
   */
  reconnect?: (attempt: number) => number | null
  now?: () => number
}

/** A second, then longer, up to ten. */
function backoff(attempt: number): number {
  return Math.min(10_000, 1000 * 2 ** (attempt - 1))
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
    cursor: { cursor: 'default', editable: false },
    notice: null,
    ended: null,
  })

  private stream: LiveStream | null = null
  private pictures: PictureSink | null = null
  private generation = 0
  private uploads = 0
  private received: number
  private resynced = 0
  private attempt = 0
  private reopening: ReturnType<typeof setTimeout> | null = null
  private panelReport: { panel: PanelSize; ratio: number; screen: PanelSize } | null = null
  /** The page closed this view; nothing reopens it. */
  private done = false

  constructor(private readonly options: LiveSessionOptions) {
    this.received = this.time()
  }

  /** What shows this view's pictures, once the view has a canvas. */
  attach(pictures: PictureSink): void {
    this.pictures?.stop()
    this.pictures = pictures
  }

  private time(): number {
    return this.options.now ? this.options.now() : performance.now()
  }

  /** The viewport of the tab this view watches, or none. */
  get viewport(): BrowserViewport | null {
    return this.state.tabs.find((tab) => tab.id === this.state.watched)?.viewport ?? null
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
    this.send({ type: 'hello', platform: this.options.platform })
    // A view that opens again takes up where the page left off.
    if (this.panelReport) {
      this.panel(this.panelReport.panel, this.panelReport.ratio, this.panelReport.screen)
    }
    if (this.state.watched) {
      this.send({ type: 'watch', tab: this.state.watched })
    }
  }

  close(): void {
    this.done = true
    if (this.reopening !== null) {
      clearTimeout(this.reopening)
      this.reopening = null
    }
    this.pictures?.stop()
    this.stream?.close()
    this.stream = null
  }

  private end(reason: string): void {
    this.pictures?.stop()
    const stream = this.stream
    this.stream = null
    stream?.close()
    this.state.ended = reason
    // A view the page closed itself has nothing to tell.
    if (!this.done) {
      this.options.onEnded?.(reason)
    }
    this.state.dialog = null
    this.state.controls = []
    const delay = this.done
      ? null
      : this.options.reconnect
        ? this.options.reconnect(this.attempt + 1)
        : backoff(this.attempt + 1)
    if (delay === null) {
      this.state.connection = 'ended'
      return
    }
    this.attempt += 1
    this.state.connection = 'opening'
    this.state.running = false
    this.state.tabs = []
    this.reopening = setTimeout(() => {
      this.reopening = null
      this.start()
    }, delay)
  }

  /**
   * The module ends a view that sends it a message the protocol refuses, so a
   * message that does not fit is this page's defect: it is reported and never
   * sent, and the view stays.
   */
  private send(message: LiveViewerMessage): void {
    const checked = liveViewerMessageSchema.safeParse(message)
    if (!checked.success) {
      reportError(`The live view built an invalid ${message.type} message`, checked.error)
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
      reportError('The live view received a frame the protocol refuses', error)
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
          this.attempt = 0
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
            const wanted = this.state.watched
            if (wanted !== null && message.tabs.some((tab) => tab.id === wanted)) {
              this.send({ type: 'watch', tab: wanted })
            } else {
              this.state.watched = message.watched
            }
          }
          break
        case 'stream':
          this.generation = message.generation
          // Video generations change independently of the watched document's controls.
          this.pictures?.start(message.generation, message.width, message.height)
          break
        case 'heartbeat':
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
    if (frame.generation !== this.generation) {
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
    if (generation === this.generation) {
      this.send({ type: 'ack', generation, sequence, decodeQueue })
    }
  }

  /** The decoder lost the stream: the next frame must be a key frame. */
  resync(): void {
    const now = this.time()
    if (now - this.resynced < 1000) {
      return
    }
    this.resynced = now
    this.send({ type: 'keyframe', generation: this.generation })
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

  panel(panel: PanelSize, devicePixelRatio: number, screen: PanelSize): void {
    this.panelReport = { panel, ratio: devicePixelRatio, screen }
    this.send({
      type: 'panel',
      width: panel.width,
      height: panel.height,
      devicePixelRatio: Math.max(0.5, Math.min(4, devicePixelRatio)),
      screenWidth: screen.width,
      screenHeight: screen.height,
    })
  }

  watch(tab: string | null): void {
    if (tab === this.state.watched) {
      return
    }
    this.state.watched = tab
    this.state.controls = []
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
