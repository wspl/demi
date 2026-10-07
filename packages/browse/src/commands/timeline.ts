// `bun browse timeline '<command>' --at 0,100,1500`: runs one command, such
// as a click, and saves the picture the page showed at each of those
// milliseconds after it, for the moments between an action and its result.
// A screenshot takes about 100 ms of its own at the page's pixel ratio, too
// long for such moments, so the browser streams every frame it paints while
// the command runs (the DevTools protocol's screencast, at CSS pixels), and
// each moment gets the frame on screen then. `--region` keeps an area of
// each frame.
import { writeFileSync } from 'node:fs'
import { CommandFailure, numeric, parse, type Context } from '../command'
import { screenOf } from '../emulation'
import { cropPng, pngSize, region, shotPath, type Clip } from '../shots'
import { readState } from '../state'
import { splitWords } from '../words'

const USAGE = 'timeline \'<command>\' --at <ms>,<ms>,... [--region x,y,w,h] [--name <prefix>]'
const OPTIONS = { at: { type: 'string' }, region: { type: 'string' }, name: { type: 'string' } } as const

/** A painted frame: when it was painted, in epoch milliseconds, its PNG, and the viewport's width in CSS pixels. */
interface Frame {
  at: number
  picture: Buffer
  width: number
}

/** How long after the last moment the stream runs, so a frame painted at that moment arrives. */
const AFTER_MS = 100

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  if (positionals.length === 0 || values.at === undefined) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const moments = values.at.split(',').map((part) => numeric(part, '--at')).sort((a, b) => a - b)
  const area: Clip | undefined = values.region === undefined ? undefined : region(values.region, '--region')
  const action = positionals.length === 1 ? splitWords(positionals[0]) : positionals
  if (action[0] === 'timeline') {
    throw new CommandFailure('A timeline runs a command other than timeline')
  }
  const prefix = values.name ?? 'timeline'
  const page = await context.browser.page()
  const cdp = await context.browser.cdp()
  const screen = screenOf(readState(context.slot).emulation ?? {})
  const frames: Frame[] = []
  const onFrame = (frame: { data: string, sessionId: number, metadata: { timestamp?: number, deviceWidth: number } }) => {
    frames.push({
      at: (frame.metadata.timestamp ?? Date.now() / 1000) * 1000,
      picture: Buffer.from(frame.data, 'base64'),
      width: frame.metadata.deviceWidth,
    })
    // The browser sends the next frame only once this one is acknowledged.
    cdp.send('Page.screencastFrameAck', { sessionId: frame.sessionId }).catch(() => undefined)
  }
  cdp.on('Page.screencastFrame', onFrame)
  let start: number
  try {
    await cdp.send('Page.startScreencast', { format: 'png' })
    // The stream sends a frame only when the page paints, so a page that
    // stays still has none: the picture before the command stands for it,
    // at CSS pixels as the stream's.
    frames.push({ at: Date.now(), picture: await context.browser.capture({ scale: 1 / screen.scale }), width: screen.width })
    await context.run(action)
    start = Date.now()
    await page.waitForTimeout((moments.at(-1) ?? 0) + AFTER_MS)
  } finally {
    cdp.off('Page.screencastFrame', onFrame)
    await cdp.send('Page.stopScreencast').catch(() => undefined)
  }
  frames.sort((a, b) => a.at - b.at)
  for (const moment of moments) {
    const shown = frames.filter((frame) => frame.at <= start + moment).at(-1)
    if (!shown) {
      throw new CommandFailure(`The browser shows no frame by ${moment} ms`)
    }
    const path = shotPath(context.slot, `${prefix}-${moment}ms`)
    writeFileSync(path, area ? await cropPng(page, shown.picture, area, shown.width) : shown.picture)
    const painted = Math.round(shown.at - start)
    context.print(`${path}  ${pngSize(path)}, the frame painted at ${painted} ms`)
  }
  if (frames.length === 1) {
    context.print('The page painted nothing new meanwhile.')
  }
}
