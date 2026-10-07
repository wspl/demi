// `bun check drag <from> <to>`: presses the mouse on one element or point,
// moves it in steps to another, and releases it there.
import { CheckFailure, numeric, parse, type Context } from '../command'
import { resolve, type Resolved } from '../find'

const USAGE = 'drag <from locator|x,y> <to locator|x,y> [--steps <n>]'
const OPTIONS = { steps: { type: 'string' } } as const

async function centre(target: Resolved): Promise<{ x: number, y: number }> {
  if (target.kind === 'point') {
    return target
  }
  const box = await target.locator.boundingBox()
  if (!box) {
    throw new CheckFailure(`${target.text} is not shown, so it has no place to drag`)
  }
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 }
}

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  if (positionals.length !== 2) {
    throw new CheckFailure(`Usage: bun check ${USAGE}`)
  }
  const steps = values.steps === undefined ? 10 : numeric(values.steps, '--steps')
  const from = await resolve(context, positionals[0])
  const to = await resolve(context, positionals[1])
  if (from.kind === 'element') {
    await from.locator.scrollIntoViewIfNeeded()
  }
  const start = await centre(from)
  const end = await centre(to)
  const page = await context.browser.page()
  await page.mouse.move(start.x, start.y)
  await page.mouse.down()
  await page.mouse.move(end.x, end.y, { steps })
  await page.mouse.up()
}
