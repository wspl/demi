// What the site's page and a demo frame say to each other: the page asks the
// frame to play a demo, and the frame says when it is ready to.
import { isDemoId, type DemoId } from '@/features'

interface PlayMessage {
  demi: 'play'
  demo: DemoId
}

/** Calls `play` for each demo the page asks for; returns the way to stop listening. */
export function readPlay(play: (demo: DemoId) => void): () => void {
  const listen = (event: MessageEvent<unknown>): void => {
    if (event.origin !== location.origin)
      return
    const data = event.data as Partial<PlayMessage> | null
    if (data?.demi === 'play' && typeof data.demo === 'string' && isDemoId(data.demo))
      play(data.demo)
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

/** Tells the page whether the desktop's tab is closed, which its address bar shows. */
export function tellTab(closed: boolean): void {
  window.parent.postMessage({ demi: 'tab', closed }, location.origin)
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
}

/** Data a command pipes from one machine close to the other, and what it is. */
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
  | { kind: 'close'; beside: ShotBeside; machines: ShotMachine[]; flow?: ShotFlow }
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

/** Asks `frame` to play `demo`. */
export function askToPlay(frame: Window, demo: DemoId): void {
  frame.postMessage({ demi: 'play', demo } satisfies PlayMessage, location.origin)
}
