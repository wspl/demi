// `demi.shot(name, { page, element, region, pad, zoom, full, now })`: saves
// a PNG of the page at its real size and pixel ratio and answers its path,
// which the call's report lists.
//
// - The page is the slot's, or `page`, such as one of `demi.webkit()`, or
//   the one `element` is on.
// - `element` (a Playwright locator) clips to an element, `region` to an
//   area in CSS pixels, and `pad` widens either.
// - `zoom` renders the area magnified rather than enlarging its pixels,
//   without changing the page's own pixel ratio.
// - A clip past the viewport fails, since the browser paints nothing there.
// - `now` takes the picture at once. Otherwise it waits first, up to two
//   seconds, for the page's transitions and finite animations to end, such
//   as a dialog that is still fading in, so the picture shows where the
//   page settles.
//
// Playwright's own screenshots go through a session of their own, which
// drops the pixel ratio the tool emulates on its session with a page until
// the page loads again. So the browser's `capture` takes every page of the
// slot's browser, and a script's `page.screenshot` or a locator's
// `screenshot` there fails, naming this helper (`routeToShot`). Any other
// page, such as WebKit's, whose context Playwright made with the slot's
// screen, is taken by Playwright, which has no zoom.
import { writeFileSync } from 'node:fs'
import type { ElementHandle, Locator, Page } from 'playwright'
import { z } from 'zod'
import { pngSize, shotPath, type Clip } from '../shots'
import { Failure, type Tool } from '../tool'

export interface ShotOptions {
  page?: Page
  element?: Locator
  region?: Clip
  pad?: number
  zoom?: number
  full?: boolean
  now?: boolean
}

/** A region as a script gives it, in CSS pixels. */
const regionSchema = z.object({ x: z.number(), y: z.number(), width: z.number().positive(), height: z.number().positive() })

/** The longest a picture waits for the page to settle; a page that never does is taken as it is. */
const SETTLE_MS = 2000

export async function shot(tool: Tool, name: string | undefined, options: ShotOptions = {}): Promise<string> {
  if (options.element !== undefined && options.region !== undefined) {
    throw new Failure('demi.shot clips to an element or to a region, not both')
  }
  if (options.full && (options.element !== undefined || options.region !== undefined)) {
    throw new Failure('demi.shot takes the full page or clips it, not both')
  }
  const slotPage = await tool.browser.page()
  // The script's `page` stands for the slot's page (call.ts, `addressed`); its main frame names the page itself.
  const page = options.element?.page() ?? options.page?.mainFrame().page() ?? slotPage
  if (options.page !== undefined && options.element !== undefined && page !== options.page.mainFrame().page()) {
    throw new Failure('demi.shot\'s element is on another page than its page')
  }
  const own = routed.has(page)
  if (!own && options.zoom !== undefined) {
    throw new Failure('demi.shot zooms only the pages of the slot\'s browser: Playwright, which takes any other, renders no page magnified')
  }
  if (!options.now) {
    await settle(page)
  }
  // A script is plain JavaScript: a region of another shape would reach the browser as its bare refusal.
  const region = options.region === undefined ? undefined : regionSchema.safeParse(options.region)
  if (region && !region.success) {
    throw new Failure(`demi.shot's region is { x, y, width, height } in CSS pixels, not ${JSON.stringify(options.region)}`)
  }
  let clip: Clip | undefined = region?.data
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
  if (clip) {
    await withinViewport(page, clip, options.element === undefined ? 'the region' : `${options.element}`)
  }
  const path = shotPath(tool.slot, name)
  writeFileSync(path, own
    ? await tool.browser.capture({ page, clip, scale: options.zoom ?? 1, full: options.full })
    // The caret stays as it shows, as in the slot's pictures.
    : await page.screenshot({ clip, fullPage: options.full, caret: 'initial' }))
  tool.wrote({ kind: 'shot', path, detail: pngSize(path) })
  return path
}

/**
 * Fails when `clip` reaches past the viewport. The browser paints only what
 * the viewport shows: past it a picture holds blank space, or, where the
 * content scrolls in a pane of its own as the gallery's does, whatever the
 * page draws there instead, such as a header over the pane. A clip of
 * `what` is checked in the viewport's CSS pixels, a pixel's rounding aside.
 */
async function withinViewport(page: Page, clip: Clip, what: string): Promise<void> {
  const [width, height] = await page.evaluate(() => [window.innerWidth, window.innerHeight])
  const right = clip.x + clip.width
  const bottom = clip.y + clip.height
  if (clip.x > -1 && clip.y > -1 && right < width + 1 && bottom < height + 1) {
    return
  }
  const area = (x: number, y: number, w: number, h: number) => `${Math.round(w)}×${Math.round(h)} at ${Math.round(x)},${Math.round(y)}`
  throw new Failure(
    `${what} spans ${area(clip.x, clip.y, clip.width, clip.height)}, past the viewport's ${width}×${height}, `
    + 'and the browser paints nothing past the viewport: take it in regions the viewport shows, '
    + 'or make the viewport larger with demi.emulate({ viewport })',
  )
}

/**
 * Waits up to two seconds for the page's finite animations and transitions
 * to end, those the next frames start included. A change the script just
 * made, such as a click that opens a drawer, starts its transition only
 * when the page draws again (Vue's Transition, two frames later), and an
 * animation that ends may start another. A hidden page draws no frame and
 * runs no transition, so it is taken as it is.
 */
async function settle(page: Page): Promise<void> {
  await page.evaluate(async (limit) => {
    if (document.visibilityState === 'hidden') {
      return
    }
    const end = performance.now() + limit
    /** Resolves when `wait` does, or at the end of the limit. */
    const until = (wait: Promise<unknown>) => new Promise<void>((resolve) => {
      const timer = setTimeout(resolve, Math.max(0, end - performance.now()))
      void wait.then(() => {
        clearTimeout(timer)
        resolve()
      })
    })
    const frame = () => until(new Promise((resolve) => requestAnimationFrame(resolve)))
    // Spinners and other endless animations never end, and a paused one waits for the page, so only finite running ones count.
    const running = () => document.getAnimations()
      .filter((animation) => animation.playState === 'running' && animation.effect?.getComputedTiming().iterations !== Infinity)
    await frame()
    await frame()
    for (let now = running(); now.length > 0 && performance.now() < end; now = running()) {
      await until(Promise.allSettled(now.map((animation) => animation.finished)))
      await frame()
    }
  }, SETTLE_MS)
}

/** Marks the locator prototype `routeToShot` routed. */
const ROUTED = Symbol('routed to demi.shot')

/** The pages of the slot's browser, which only `demi.shot` takes. */
const routed = new WeakSet<Page>()

/** The failure of a Playwright screenshot of a page of the slot's browser (module comment). */
const REFUSED = 'A Playwright screenshot of the slot\'s browser drops the page\'s pixel ratio until the page loads again: '
  + 'take it with demi.shot(name, { page, element, region, full })'

/** Whether the element handles' prototype is routed, or being routed. */
let handlesRouted = false

/**
 * Makes `page.screenshot`, and the `screenshot` of every locator and element
 * handle on `page`, fail with what to use instead (module comment).
 */
export function routeToShot(page: Page): void {
  routed.add(page)
  page.screenshot = () => Promise.reject(new Failure(REFUSED))
  if (!handlesRouted) {
    handlesRouted = true
    // A page that closes as it opens leaves the routing to the next one.
    routeHandles(page).catch(() => {
      handlesRouted = false
    })
  }
  // Every locator shares one prototype, which is routed once; one on another page, such as WebKit's, keeps its own.
  const prototype = Object.getPrototypeOf(page.locator('html')) as Locator & { [ROUTED]?: true }
  if (prototype[ROUTED]) {
    return
  }
  const own = prototype.screenshot
  prototype.screenshot = function (this: Locator, options) {
    return routed.has(this.page()) ? Promise.reject(new Failure(REFUSED)) : own.call(this, options)
  }
  prototype[ROUTED] = true
}

/**
 * Routes the prototype every element handle shares, which only a handle of
 * a page's element reaches: the call makes one before its script can.
 */
async function routeHandles(page: Page): Promise<void> {
  const handle = await page.evaluateHandle(() => document.documentElement)
  const prototype = Object.getPrototypeOf(handle) as ElementHandle
  await handle.dispose()
  const own = prototype.screenshot
  prototype.screenshot = async function (this: ElementHandle, options) {
    const frame = await this.ownerFrame()
    if (frame && routed.has(frame.page())) {
      throw new Failure(REFUSED)
    }
    return own.call(this, options)
  }
}
