// `demi.timeline(action, [0, 100, 1500], { region, name })`: runs the
// action, such as a click, and saves the picture the page showed at each of
// those milliseconds after it, for the moments between an action and its
// result; answers their paths. A screenshot takes about 100 ms of its own at
// the page's pixel ratio, too long for such moments, so the browser streams
// every frame it paints while the action runs (the DevTools protocol's
// screencast, at CSS pixels), and each moment gets the frame on screen then.
// `region` keeps an area of each frame, in CSS pixels.
import { writeFileSync } from 'node:fs'
import { screenOf } from '../emulation'
import { cropPng, pngSize, shotPath, type Clip } from '../shots'
import { readState } from '../state'
import { Failure, type Tool } from '../tool'

export interface TimelineOptions {
  region?: Clip
  /** What the pictures' names start with, `timeline` unless given. */
  name?: string
}

/** A painted frame: when it was painted, in epoch milliseconds, its PNG, and the viewport's width in CSS pixels. */
interface Frame {
  at: number
  picture: Buffer
  width: number
}

/** How long after the last moment the stream runs, so a frame painted at that moment arrives. */
const AFTER_MS = 100

export async function timeline(
  tool: Tool,
  action: () => unknown,
  moments: number[],
  options: TimelineOptions = {},
): Promise<string[]> {
  if (typeof action !== 'function' || moments.length === 0) {
    throw new Failure('demi.timeline takes an action to run and the milliseconds after it to keep, such as demi.timeline(() => button.click(), [0, 100, 1500])')
  }
  const sorted = [...moments].sort((a, b) => a - b)
  const prefix = options.name ?? 'timeline'
  const page = await tool.browser.page()
  const cdp = await tool.browser.cdp()
  const screen = screenOf(readState(tool.slot).emulation ?? {})
  const frames: Frame[] = []
  const onFrame = (frame: { data: string, sessionId: number, metadata: { timestamp?: number, deviceWidth: number } }) => {
    frames.push({
      at: (frame.metadata.timestamp ?? Date.now() / 1000) * 1000,
      picture: Buffer.from(frame.data, 'base64'),
      width: frame.metadata.deviceWidth,
    })
    // The browser sends the next frame only once this one is acknowledged;
    // a frame the stream sends as it stops needs no answer.
    cdp.send('Page.screencastFrameAck', { sessionId: frame.sessionId }).catch(() => undefined)
  }
  cdp.on('Page.screencastFrame', onFrame)
  let start: number
  try {
    await cdp.send('Page.startScreencast', { format: 'png' })
    // The stream sends a frame only when the page paints, so a page that
    // stays still has none: the picture before the action stands for it,
    // at CSS pixels as the stream's.
    frames.push({ at: Date.now(), picture: await tool.browser.capture({ scale: 1 / screen.scale }), width: screen.width })
    await action()
    start = Date.now()
    await page.waitForTimeout((sorted.at(-1) ?? 0) + AFTER_MS)
  } finally {
    cdp.off('Page.screencastFrame', onFrame)
    // A page that closed during the action has no stream left to stop.
    await cdp.send('Page.stopScreencast').catch(() => undefined)
  }
  frames.sort((a, b) => a.at - b.at)
  const paths: string[] = []
  for (const moment of sorted) {
    const shown = frames.filter((frame) => frame.at <= start + moment).at(-1)
    if (!shown) {
      throw new Failure(`The browser shows no frame by ${moment} ms`)
    }
    const path = shotPath(tool.slot, `${prefix}-${moment}ms`)
    writeFileSync(path, options.region ? await cropPng(page, shown.picture, options.region, shown.width) : shown.picture)
    const painted = Math.round(shown.at - start)
    const still = frames.length === 1 ? ', the page painted nothing new' : ''
    tool.wrote({ kind: 'timeline', path, detail: `${pngSize(path)}, painted at ${painted} ms${still}` })
    paths.push(path)
  }
  return paths
}
