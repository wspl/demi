// `bun browse hover <locator|x,y>`: moves the pointer over an element or a
// point and leaves it there, for tooltips and hover states.
import { CommandFailure, parse, type Context } from '../command'
import { resolve, uncovered } from '../find'

const USAGE = 'hover <locator|x,y>'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  if (positionals.length !== 1) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const target = await resolve(context, positionals[0])
  if (target.kind === 'point') {
    const page = await context.browser.page()
    await page.mouse.move(target.x, target.y)
    return
  }
  await uncovered(context, target.locator, target.text, 'pointer')
  await target.locator.hover({ timeout: 10_000 })
}
