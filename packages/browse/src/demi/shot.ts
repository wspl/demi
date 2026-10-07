// `demi.shot(name, { element, region, pad, zoom, full, now })`: saves a PNG
// of the page at its real size and pixel ratio and answers its path, which
// the call's report lists. `element` (a Playwright locator) clips to an
// element, `region` to an area in CSS pixels, `pad` widens either, and
// `zoom` renders the area magnified rather than enlarging its pixels,
// without changing the page's own pixel ratio. It waits first, up to two
// seconds, for the page's transitions and finite animations to end, such as
// a dialog that is still fading in, so the picture shows where the page
// settles; `now` takes it at once.
//
// Playwright's own screenshots go through a session of their own, which
// drops the pixel ratio the tool emulates on its session with the page, so
// the browser's `capture` takes them instead.
import { writeFileSync } from 'node:fs'
import type { Locator } from 'playwright'
import { pngSize, shotPath, type Clip } from '../shots'
import { Failure, type Tool } from '../tool'

export interface ShotOptions {
  element?: Locator
  region?: Clip
  pad?: number
  zoom?: number
  full?: boolean
  now?: boolean
}

/** The longest a picture waits for the page to settle; a page that never does is taken as it is. */
const SETTLE_MS = 2000

export async function shot(tool: Tool, name: string | undefined, options: ShotOptions = {}): Promise<string> {
  if (options.element !== undefined && options.region !== undefined) {
    throw new Failure('demi.shot clips to an element or to a region, not both')
  }
  if (options.full && (options.element !== undefined || options.region !== undefined)) {
    throw new Failure('demi.shot takes the full page or clips it, not both')
  }
  if (!options.now) {
    await settle(tool)
  }
  let clip = options.region
  if (options.element !== undefined) {
    await options.element.scrollIntoViewIfNeeded()
    const box = await options.element.boundingBox()
    if (!box) {
      throw new Failure(`${options.element} is not shown, so it has no picture`)
    }
    clip = box
  }
  const pad = options.pad ?? 0
  if (clip && pad > 0) {
    clip = { x: Math.max(0, clip.x - pad), y: Math.max(0, clip.y - pad), width: clip.width + 2 * pad, height: clip.height + 2 * pad }
  }
  const path = shotPath(tool.slot, name)
  writeFileSync(path, await tool.browser.capture({ clip, scale: options.zoom ?? 1, full: options.full }))
  tool.wrote({ kind: 'shot', path, detail: pngSize(path) })
  return path
}

/** Waits up to two seconds for the page's finite animations and transitions to end. */
async function settle(tool: Tool): Promise<void> {
  const page = await tool.browser.page()
  // Spinners and other endless animations never end, so only finite ones are waited for.
  await page.evaluate(async (limit) => {
    const finite = document.getAnimations().filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity)
    await Promise.race([
      Promise.allSettled(finite.map((animation) => animation.finished)),
      new Promise((resolve) => setTimeout(resolve, limit)),
    ])
  }, SETTLE_MS)
}
