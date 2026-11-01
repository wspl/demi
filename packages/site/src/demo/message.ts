// What the site's page and a demo frame say to each other: the page asks the
// frame to play a demo, and to pause it and go on, and the frame says when it
// is ready to.
import { isDemoId, type DemoId } from '@/features'

interface PlayMessage {
  demi: 'play'
  demo: DemoId
}

interface PauseMessage {
  demi: 'pause'
  paused: boolean
}

/**
 * Calls `play` for each demo the page asks for, and `pause` each time it
 * pauses the play or lets it go on; returns the way to stop listening.
 */
export function readPlay(play: (demo: DemoId) => void, pause?: (paused: boolean) => void): () => void {
  const listen = (event: MessageEvent<unknown>): void => {
    if (event.origin !== location.origin)
      return
    const data = event.data as Partial<PlayMessage> | Partial<PauseMessage> | null
    if (data?.demi === 'play' && 'demo' in data && typeof data.demo === 'string' && isDemoId(data.demo))
      play(data.demo)
    if (data?.demi === 'pause' && 'paused' in data && typeof data.paused === 'boolean')
      pause?.(data.paused)
  }
  window.addEventListener('message', listen)
  return () => window.removeEventListener('message', listen)
}

/** Tells the page the frame can play. */
export function tellReady(): void {
  window.parent.postMessage({ demi: 'ready' }, location.origin)
}

/** A part of a frame's screen, in the screen's CSS pixels. */
export interface CameraRect {
  x: number
  y: number
  width: number
  height: number
}

/** How long the camera takes to come close or go back: a scene waits for it before what it shows begins. */
export const CAMERA_MS = 1000

/** Where the camera points: the part of the screen it brings close, none for all of it, and whether it cuts there rather than moves. */
export interface CameraShot {
  rect: CameraRect | null
  cut: boolean
}

/** Tells the page where the camera points. */
export function tellCamera(rect: CameraRect | null, cut = false): void {
  window.parent.postMessage({ demi: 'camera', rect, cut } satisfies CameraShot & { demi: string }, location.origin)
}

/** Answers where a `camera` message points, `undefined` for another message. */
export function cameraOf(data: unknown): CameraShot | undefined {
  const message = data as { demi?: string; rect?: CameraRect | null; cut?: boolean } | null
  if (message?.demi !== 'camera')
    return undefined
  return { rect: message.rect ?? null, cut: message.cut === true }
}

/** A machine the stage can show beside the app: the agent's work reaching it. */
export type ShotDevice = 'macbook' | 'windows-pc' | 'vps' | 'home-lab'

/**
 * Which app a machine close stands beside: the browser, at the desk, where
 * the phone stood; or the phone, away from the desk, where the browser stood.
 */
export type ShotBeside = 'browser' | 'phone'

/** A machine close: what its terminal prints, and whether its work is done. */
export interface ShotMachine {
  device: ShotDevice
  lines: string[]
  done: boolean
  /**
   * In the square of every machine beside the browser: whether the
   * conversation runs on it, and whether it waits, doing nothing for the
   * conversation now.
   */
  here?: boolean
  idle?: boolean
}

/** Data a command pipes from one machine close to another, and what it is. */
export interface ShotFlow {
  from: ShotDevice
  to: ShotDevice
  label: string
}

/**
 * What the stage shows besides the browser and the phone for a moment: the
 * machines the conversation works on now, close, beside one of them, and the
 * data flowing between them; every machine at once, around the app, for the
 * scene's end; or nothing at all, as a demo that loops starts over.
 */
export type DeviceShot =
  | { kind: 'close'; beside: ShotBeside; machines: ShotMachine[]; flows?: ShotFlow[] }
  | { kind: 'all' }
  | { kind: 'clear' }

/** Tells the page what the stage shows beside the app; none for the app alone. */
export function tellShot(shot: DeviceShot | null): void {
  window.parent.postMessage({ demi: 'shot', shot }, location.origin)
}

/** Answers what a `shot` message shows, `undefined` for another message. */
export function shotOf(data: unknown): DeviceShot | null | undefined {
  const message = data as { demi?: string; shot?: DeviceShot | null } | null
  if (message?.demi !== 'shot')
    return undefined
  return message.shot ?? null
}

/**
 * A part of Demi's runtime, as the stage's card draws it: the programs the
 * model's one tool, the shell, runs; the `demi` commands it runs for the
 * browser, other machines and other agents; and the compaction of the
 * context the model reads.
 */
export type RuntimePart = 'programs' | 'browser' | 'host' | 'agent' | 'compact'

/**
 * Which devices the stage of several shows (`AnywhereStage.vue`): the
 * browser, which sends the task; the phone, come in from the right to take
 * it over, the browser pushed aside smaller; the tablet, come in from the
 * left, and the phone working together with the browser gone; and the
 * browser back from above as the two go down and away.
 */
export type DevicesShot = 'browser' | 'phone' | 'both' | 'back'

/** One of the abilities the pieces card holds, which a request snaps together. */
export type Piece = 'web' | 'search' | 'write' | 'speech' | 'git' | 'file' | 'schedule' | 'notify'

/** Pieces snapped together for one request, and how many of them are in place yet. */
export interface PieceRecipe {
  name: string
  pieces: Piece[]
  placed: number
}

/**
 * What the stage shows around the app, drawing what the app cannot show:
 * Demi's runtime on a card at the browser's corner, with the part the agent
 * uses now, none between steps, and how full its context is, 0 to 1; how
 * the stage of several devices frames them, with the machines the
 * conversation has reached and the one its work runs on now; or how many of
 * a short film's production stages are done (the storyboard, the voiceover
 * and the music, a clip from each shot, the cut), and whether the next runs;
 * or the pieces of Demi's abilities and the recipes requests snapped them
 * into, with the piece of the first recipe that runs now.
 */
export type StageCard =
  | { kind: 'runtime'; part: RuntimePart | null; context: number }
  | { kind: 'devices'; shot: DevicesShot; machines: ShotDevice[]; using: ShotDevice | null }
  | { kind: 'production'; done: number; running: boolean }
  | { kind: 'pieces'; recipes: PieceRecipe[]; lit: number | null }

/** Tells the page what the corner card shows; none takes the card away. */
export function tellCard(card: StageCard | null): void {
  window.parent.postMessage({ demi: 'card', card }, location.origin)
}

/** Answers what a `card` message shows, `undefined` for another message. */
export function cardOf(data: unknown): StageCard | null | undefined {
  const message = data as { demi?: string; card?: StageCard | null } | null
  if (message?.demi !== 'card')
    return undefined
  return message.card ?? null
}

/** Asks `frame` to play `demo`. */
export function askToPlay(frame: Window, demo: DemoId): void {
  frame.postMessage({ demi: 'play', demo } satisfies PlayMessage, location.origin)
}

/** Asks `frame` to pause its play where it is, or to go on from there. */
export function askToPause(frame: Window, paused: boolean): void {
  frame.postMessage({ demi: 'pause', paused } satisfies PauseMessage, location.origin)
}
