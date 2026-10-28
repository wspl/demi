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

/** Asks `frame` to play `demo`. */
export function askToPlay(frame: Window, demo: DemoId): void {
  frame.postMessage({ demi: 'play', demo } satisfies PlayMessage, location.origin)
}
