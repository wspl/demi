// `bun browse drag <from> <to>`: presses the mouse on one element or point,
// moves it in steps to another, and releases it there. `--hold` keeps the
// button down at the end, for a look at the drag in progress, and `drag
// --release [<to>]` moves on and lets go, or `press Escape` cancels it.
import { CommandFailure, numeric, parse, type Context } from '../command'
import { resolve, type Resolved } from '../find'

const USAGE = 'drag <from locator|x,y> <to locator|x,y> [--steps <n>] [--hold] | drag --release [<to locator|x,y>]'
const OPTIONS = { steps: { type: 'string' }, hold: { type: 'boolean' }, release: { type: 'boolean' } } as const

async function centre(target: Resolved): Promise<{ x: number, y: number }> {
  if (target.kind === 'point') {
    return target
  }
  const box = await target.locator.boundingBox()
  if (!box) {
    throw new CommandFailure(`${target.text} is not shown, so it has no place to drag`)
  }
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 }
}

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  const steps = values.steps === undefined ? 10 : numeric(values.steps, '--steps')
  if (values.release) {
    if (positionals.length > 1) {
      throw new CommandFailure(`Usage: bun browse ${USAGE}`)
    }
    const page = await context.browser.page()
    if (positionals[0] !== undefined) {
      const end = await centre(await resolve(context, positionals[0]))
      await page.mouse.move(end.x, end.y, { steps })
    }
    await page.mouse.up()
    return
  }
  if (positionals.length !== 2) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
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
  if (!values.hold) {
    await page.mouse.up()
  }
}
