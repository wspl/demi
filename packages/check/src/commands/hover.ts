// `bun check hover <locator|x,y>`: moves the pointer over an element or a
// point and leaves it there, for tooltips and hover states.
import { CheckFailure, parse, type Context } from '../command'
import { resolve } from '../find'

const USAGE = 'hover <locator|x,y>'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  if (positionals.length !== 1) {
    throw new CheckFailure(`Usage: bun check ${USAGE}`)
  }
  const target = await resolve(context, positionals[0])
  if (target.kind === 'point') {
    const page = await context.browser.page()
    await page.mouse.move(target.x, target.y)
    return
  }
  await target.locator.hover({ timeout: 10_000 })
}
