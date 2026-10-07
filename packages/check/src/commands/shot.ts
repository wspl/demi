// `bun check shot [name]`: saves a PNG of the page at its real size and
// pixel ratio and prints its path, for the report to list. `--element`
// clips to an element, `--region` to an area, `--pad` widens either, and
// `--zoom` renders the area magnified rather than enlarging its pixels.
// Animations under way are finished first, so the shot shows where they end.
import { writeFileSync } from 'node:fs'
import { CheckFailure, numeric, parse, type Context } from '../command'
import { launchOptions } from '../browser'
import { element } from '../find'
import { readState } from '../state'
import { pngSize, shotPath, zoomed, type Clip } from '../shots'

const USAGE = 'shot [name|path.png] [--element <locator>] [--region x,y,w,h] [--pad <px>] [--zoom <n>] [--full]'
const OPTIONS = {
  element: { type: 'string' },
  region: { type: 'string' },
  pad: { type: 'string' },
  zoom: { type: 'string' },
  full: { type: 'boolean' },
} as const

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  if (positionals.length > 1 || (values.element !== undefined && values.region !== undefined)) {
    throw new CheckFailure(`Usage: bun check ${USAGE}`)
  }
  const page = await context.browser.page()
  const path = shotPath(context.slot, positionals[0])
  const pad = values.pad === undefined ? 0 : numeric(values.pad, '--pad')
  let clip: Clip | undefined
  if (values.region !== undefined) {
    clip = region(values.region)
  } else if (values.element !== undefined) {
    const target = await element(context, values.element)
    await target.scrollIntoViewIfNeeded({ timeout: 10_000 })
    const box = await target.boundingBox()
    if (!box) {
      throw new CheckFailure(`${values.element} is not shown, so it has no picture`)
    }
    clip = box
  }
  if (clip && pad > 0) {
    clip = { x: Math.max(0, clip.x - pad), y: Math.max(0, clip.y - pad), width: clip.width + 2 * pad, height: clip.height + 2 * pad }
  }
  if (values.zoom !== undefined) {
    const zoom = numeric(values.zoom, '--zoom')
    writeFileSync(path, await zoomed(page, await context.browser.cdp(), clip, zoom, launchOptions(readState(context.slot).emulation ?? {}).isMobile ?? false))
  } else {
    // A shot shows where the page settles: a menu or a popover still fading in is finished first. The
    // moments in between are `timeline`'s.
    await page.screenshot({ path, clip, fullPage: values.full, animations: 'disabled' })
  }
  context.print(`${path}  ${pngSize(path)}`)
}

function region(text: string): Clip {
  const parts = text.split(',').map((part) => numeric(part, '--region'))
  if (parts.length !== 4) {
    throw new CheckFailure(`--region takes x,y,width,height, not ${text}`)
  }
  const [x, y, width, height] = parts
  return { x, y, width, height }
}
