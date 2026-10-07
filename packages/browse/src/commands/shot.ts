// `bun browse shot [name]`: saves a PNG of the page at its real size and
// pixel ratio and prints its path, for the report to list. `--element`
// clips to an element, `--region` to an area, `--pad` widens either, and
// `--zoom` renders the area magnified rather than enlarging its pixels,
// without changing the page's own pixel ratio. It waits first for the
// page's transitions and finite animations to end, such as a dialog that is
// still fading in, so the picture shows where the page settles; `--now`
// takes it at once, as `timeline` does its frames.
import { writeFileSync } from 'node:fs'
import { CommandFailure, numeric, parse, type Context } from '../command'
import { element } from '../find'
import { pngSize, region, shotPath, type Clip } from '../shots'

const USAGE = 'shot [name|path.png] [--element <locator>] [--region x,y,w,h] [--pad <px>] [--zoom <n>] [--full] [--now]'
const OPTIONS = {
  element: { type: 'string' },
  region: { type: 'string' },
  pad: { type: 'string' },
  zoom: { type: 'string' },
  full: { type: 'boolean' },
  now: { type: 'boolean' },
} as const

/** The longest a picture waits for the page to settle; a page that never does is taken as it is. */
const SETTLE_MS = 2000

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  const clipped = values.element !== undefined || values.region !== undefined
  if (positionals.length > 1 || (values.element !== undefined && values.region !== undefined) || (values.full && clipped)) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const page = await context.browser.page()
  if (!values.now) {
    // Spinners and other endless animations never end, so only finite ones are waited for.
    await page.evaluate(async (limit) => {
      const finite = document.getAnimations().filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity)
      await Promise.race([
        Promise.allSettled(finite.map((animation) => animation.finished)),
        new Promise((resolve) => setTimeout(resolve, limit)),
      ])
    }, SETTLE_MS)
  }
  const path = shotPath(context.slot, positionals[0])
  const pad = values.pad === undefined ? 0 : numeric(values.pad, '--pad')
  let clip: Clip | undefined
  if (values.region !== undefined) {
    clip = region(values.region, '--region')
  } else if (values.element !== undefined) {
    const target = await element(context, values.element)
    await target.scrollIntoViewIfNeeded({ timeout: 10_000 })
    const box = await target.boundingBox()
    if (!box) {
      throw new CommandFailure(`${values.element} is not shown, so it has no picture`)
    }
    clip = box
  }
  if (clip && pad > 0) {
    clip = { x: Math.max(0, clip.x - pad), y: Math.max(0, clip.y - pad), width: clip.width + 2 * pad, height: clip.height + 2 * pad }
  }
  const scale = values.zoom === undefined ? 1 : numeric(values.zoom, '--zoom')
  writeFileSync(path, await context.browser.capture({ clip, scale, full: values.full }))
  context.print(`${path}  ${pngSize(path)}`)
}
