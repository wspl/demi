import { z } from 'zod'
import {
  browserCreatedBySchema,
  browserViewportModeSchema,
  browserViewportSchema,
} from './index'

/**
 * The live view's protocol (`browser-live-view.md` § The stream): what the
 * page and the live view module say to each other over a user stream. The
 * stream carries bytes without boundaries, so each message is one frame: a
 * four-byte big-endian length of the rest, a one-byte kind, then its payload.
 * The page and the module ship in the same release; there is no version.
 */
export const LIVE_FRAME_KIND = {
  /** UTF-8 JSON of a control message. */
  control: 1,
  /** A video frame: the header below, then H.264 Annex B data. */
  video: 2,
  /** A chosen file's bytes: the upload and file index, then the data. */
  file: 3,
} as const
/** The largest frame after its length: a paste's text and HTML, or a key frame. */
export const LIVE_MAX_FRAME_BYTES = 16 * 1024 * 1024
/** A file frame's largest data. */
export const LIVE_FILE_CHUNK_BYTES = 64 * 1024
/** How often the module speaks at least, so a still page is told from a stall. */
export const LIVE_HEARTBEAT_MS = 250
/** Silence after which the page shows the stream as stalled and stops sending input. */
export const LIVE_STALL_MS = 1000

/**
 * A video frame's header, big-endian: the tab ID (24 ASCII bytes), the
 * stream generation (u32), the sequence number (u32), flags (u8, 1 = key
 * frame), three reserved bytes, the timestamp in microseconds (f64), and the
 * picture's width and height in pixels (u16 each).
 */
export const LIVE_VIDEO_HEADER_BYTES = 48
/** A file frame's header: the upload (u32) and the file's index in it (u32). */
export const LIVE_FILE_HEADER_BYTES = 8

const tab = z.string().regex(/^t_[A-Za-z0-9_-]{22}$/)
const cssLength = z.number().int().min(1).max(4096)
const coordinate = z.number().min(0).max(4096)
/** The modifier keys held, as CDP numbers them: Alt 1, Control 2, Meta 4, Shift 8. */
const modifiers = z.number().int().min(0).max(15)
const generation = z.number().int().nonnegative()
/** A control's identity in its page, as `crypto.randomUUID` makes it. */
const controlToken = z.string().regex(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/)
const LIVE_TEXT_BYTES = 1_000_000
const LIVE_HTML_BYTES = 4_000_000

/** A native form control an observer reports (`browser-live-view.md` § Input). */
export const liveControlOptionSchema = z.strictObject({
  label: z.string().max(2000),
  value: z.string().max(2000),
  group: z.string().max(2000),
  disabled: z.boolean(),
  hidden: z.boolean(),
  selected: z.boolean(),
})
export const liveControlSchema = z.strictObject({
  token: controlToken,
  revision: z.number().int().nonnegative(),
  kind: z.enum(['select', 'date', 'month', 'week', 'time', 'datetime-local', 'color', 'suggestions', 'file']),
  label: z.string().max(2000),
  value: z.string().max(10_000),
  min: z.string().max(100),
  max: z.string().max(100),
  step: z.string().max(100),
  accept: z.string().max(1000),
  multiple: z.boolean(),
  disabled: z.boolean(),
  required: z.boolean(),
  size: z.number().int().nonnegative(),
  options: z.array(liveControlOptionSchema).max(1000),
  rect: z.strictObject({
    x: z.number(),
    y: z.number(),
    width: z.number().positive(),
    height: z.number().positive(),
  }),
})
export type LiveControl = z.infer<typeof liveControlSchema>

export const liveTabSchema = z.strictObject({
  id: tab,
  title: z.string(),
  url: z.string(),
  createdBy: browserCreatedBySchema,
  viewport: browserViewportSchema,
})
export type LiveTab = z.infer<typeof liveTabSchema>
/** A watched tab's viewport: its CSS size, its pixel ratio and its mode. */
export type LiveViewport = LiveTab['viewport']

export const liveDialogSchema = z.strictObject({
  type: z.enum(['alert', 'confirm', 'prompt', 'beforeunload']),
  message: z.string(),
  defaultText: z.string(),
})
export type LiveDialog = z.infer<typeof liveDialogSchema>

/** What the page sends. */
export const liveViewerMessageSchema = z.union([
  /** First: the viewer's platform, which decides how its keys map on the Host. */
  z.strictObject({ type: z.literal('hello'), platform: z.enum(['mac', 'windows', 'linux', 'other']) }),
  /** The panel's size in CSS pixels and the viewer's screen. */
  z.strictObject({
    type: z.literal('panel'),
    width: cssLength,
    height: cssLength,
    devicePixelRatio: z.number().min(0.5).max(4),
    screenWidth: cssLength,
    screenHeight: cssLength,
  }),
  /** The tab the view shows, or none. */
  z.strictObject({ type: z.literal('watch'), tab: tab.nullable() }),
  z.strictObject({ type: z.literal('mode'), tab, mode: browserViewportModeSchema.exclude(['custom']) }),
  z.strictObject({
    type: z.literal('pointer'),
    tab,
    action: z.enum(['move', 'down', 'up']),
    x: coordinate,
    y: coordinate,
    button: z.enum(['none', 'left', 'middle', 'right']),
    buttons: z.number().int().min(0).max(31),
    clickCount: z.number().int().min(0).max(3),
    modifiers,
  }),
  z.strictObject({
    type: z.literal('wheel'),
    tab,
    x: coordinate,
    y: coordinate,
    deltaX: z.number().min(-10_000).max(10_000),
    deltaY: z.number().min(-10_000).max(10_000),
    modifiers,
  }),
  z.strictObject({
    type: z.literal('key'),
    tab,
    action: z.enum(['down', 'up']),
    key: z.string().max(64),
    code: z.string().max(64),
    keyCode: z.number().int().min(0).max(255),
    modifiers,
    repeat: z.boolean(),
    location: z.number().int().min(0).max(3),
    text: z.string().min(1).max(16).optional(),
    altGraph: z.boolean(),
  }),
  /** Committed text: an input method's result. */
  z.strictObject({ type: z.literal('text'), tab, text: z.string().max(20_000) }),
  /** An input method's text being composed. */
  z.strictObject({ type: z.literal('composition'), tab, text: z.string().max(20_000) }),
  z.strictObject({ type: z.literal('paste'), tab, text: z.string().max(LIVE_TEXT_BYTES), html: z.string().max(LIVE_HTML_BYTES) }),
  /** A choice in a native control, for the revision the viewer saw. */
  z.strictObject({
    type: z.literal('choice'),
    tab,
    token: controlToken,
    revision: z.number().int().nonnegative(),
    value: z.string().max(10_000),
    indices: z.array(z.number().int().nonnegative()).max(1000),
  }),
  /** Files chosen for a file input; their bytes follow as file frames. */
  z.strictObject({
    type: z.literal('upload'),
    tab,
    token: controlToken,
    revision: z.number().int().nonnegative(),
    upload: z.number().int().nonnegative(),
    files: z.array(z.strictObject({
      name: z.string().min(1).max(255).regex(/^[^/\\\0]+$/),
      mimeType: z.string().max(200),
      size: z.number().int().nonnegative(),
    })).max(100),
  }),
  z.strictObject({ type: z.literal('dialog'), tab, accept: z.boolean(), text: z.string().max(2000).optional() }),
  /** The page showed this frame; the module paces itself by these. */
  z.strictObject({ type: z.literal('ack'), generation, sequence: z.number().int().nonnegative(), decodeQueue: z.number().int().nonnegative() }),
  /** The decoder lost the stream; the next frame must be a key frame. */
  z.strictObject({ type: z.literal('keyframe'), generation }),
  /** Release every key and button this viewer holds. */
  z.strictObject({ type: z.literal('release') }),
])
export type LiveViewerMessage = z.infer<typeof liveViewerMessageSchema>

/** What the module sends. */
export const liveModuleMessageSchema = z.union([
  /** The browser's tabs and the one this viewer watches. */
  z.strictObject({ type: z.literal('state'), running: z.boolean(), tabs: z.array(liveTabSchema), watched: tab.nullable() }),
  /** Video frames of this generation follow, starting with a key frame. */
  z.strictObject({ type: z.literal('stream'), tab, generation, width: z.number().int().positive(), height: z.number().int().positive() }),
  z.strictObject({ type: z.literal('heartbeat') }),
  z.strictObject({ type: z.literal('cursor'), tab, cursor: z.string().max(200), editable: z.boolean() }),
  z.strictObject({ type: z.literal('controls'), tab, controls: z.array(liveControlSchema).max(100) }),
  /** Text the watched tab copied shortly after this viewer's input. */
  z.strictObject({ type: z.literal('clipboard'), text: z.string().max(LIVE_TEXT_BYTES) }),
  z.strictObject({ type: z.literal('dialog'), tab, dialog: liveDialogSchema.nullable() }),
  /** Whether a choice or an upload reached its control. */
  z.strictObject({ type: z.literal('choice'), token: controlToken, accepted: z.boolean() }),
  /** Something the viewer asked for failed; the stream goes on. */
  z.strictObject({ type: z.literal('notice'), code: z.string(), message: z.string() }),
  /** The browser ended; the stream ends after this. */
  z.strictObject({ type: z.literal('ended'), reason: z.enum(['browser_ended', 'released']) }),
])
export type LiveModuleMessage = z.infer<typeof liveModuleMessageSchema>
