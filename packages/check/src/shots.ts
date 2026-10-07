// Where screenshots go: `.cache/check/shots/<name>.png` in the slot, or the
// path a name gives when it has a folder in it, such as a report's folder
// outside the repository.
import { mkdirSync, readFileSync } from 'node:fs'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import type { CDPSession, Page } from 'playwright'
import { slotPaths, type Slot } from './slot'

export type Clip = { x: number, y: number, width: number, height: number }

export function shotPath(slot: Slot, name?: string): string {
  const chosen = name ?? `shot-${new Date().toISOString().slice(11, 23).replaceAll(':', '-')}`
  const file = chosen.endsWith('.png') ? chosen : `${chosen}.png`
  const path = file.includes('/')
    ? (isAbsolute(file) ? file : resolve(slot.root, file))
    : join(slotPaths(slot).shots, file)
  mkdirSync(dirname(path), { recursive: true })
  return path
}

/** The pixel size the header of the PNG at `path` gives, as `2880×1800 px`. */
export function pngSize(path: string): string {
  const header = readFileSync(path).subarray(0, 24)
  return `${header.readUInt32BE(16)}×${header.readUInt32BE(20)} px`
}

/** What the page's screen is like, as the emulation the browser holds says. */
interface Metrics {
  ratio: number
  screenWidth: number
  screenHeight: number
  orientation: string
  angle: number
}

/**
 * A PNG of `clip`, or of the viewport, rendered at the page's pixel ratio
 * times `zoom`: Playwright shoots only at the page's own ratio, and the
 * DevTools protocol renders a clip at any scale, counted in CSS pixels.
 * `mobile` is whether the page emulates a phone.
 */
export async function zoomed(page: Page, cdp: CDPSession, clip: Clip | undefined, zoom: number, mobile: boolean): Promise<Buffer> {
  const metrics: Metrics = await page.evaluate(() => ({
    ratio: devicePixelRatio,
    screenWidth: screen.width,
    screenHeight: screen.height,
    orientation: screen.orientation.type,
    angle: screen.orientation.angle,
  }))
  const viewport = page.viewportSize() ?? { width: 0, height: 0 }
  const { data } = await cdp.send('Page.captureScreenshot', {
    format: 'png',
    clip: { ...clip ?? { x: 0, y: 0, ...viewport }, scale: metrics.ratio * zoom },
  })
  // The scaled capture ends by dropping the page's emulated metrics, its
  // pixel ratio among them, which Playwright set and does not know are
  // gone: they are set again as they were.
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    ...viewport,
    deviceScaleFactor: metrics.ratio,
    mobile,
    screenWidth: metrics.screenWidth,
    screenHeight: metrics.screenHeight,
    screenOrientation: { type: orientationType(metrics.orientation), angle: metrics.angle },
  })
  return Buffer.from(data, 'base64')
}

type Orientation = 'portraitPrimary' | 'portraitSecondary' | 'landscapePrimary' | 'landscapeSecondary'

/** The protocol's name of a screen orientation the page names, such as `landscape-primary`. */
function orientationType(type: string): Orientation {
  const names: Record<string, Orientation> = {
    'portrait-primary': 'portraitPrimary',
    'portrait-secondary': 'portraitSecondary',
    'landscape-primary': 'landscapePrimary',
    'landscape-secondary': 'landscapeSecondary',
  }
  return names[type] ?? 'landscapePrimary'
}
