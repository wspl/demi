// `bun check scroll <locator|x,y> [dx] [dy]`: without amounts, scrolls an
// element into view; with them, turns the mouse wheel by dx,dy pixels over
// the element or the point, as a trackpad does.
import { CheckFailure, numeric, parse, type Context } from '../command'
import { resolve } from '../find'

const USAGE = 'scroll <locator|x,y> [dx] [dy]'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  const [where, dx, dy] = positionals
  if (where === undefined || positionals.length > 3) {
    throw new CheckFailure(`Usage: bun check ${USAGE}`)
  }
  const target = await resolve(context, where)
  const page = await context.browser.page()
  if (dx === undefined) {
    if (target.kind === 'point') {
      throw new CheckFailure('A point needs amounts to scroll by: scroll x,y dx dy')
    }
    await target.locator.scrollIntoViewIfNeeded({ timeout: 10_000 })
    return
  }
  if (target.kind === 'point') {
    await page.mouse.move(target.x, target.y)
  } else {
    await target.locator.hover({ timeout: 10_000 })
  }
  await page.mouse.wheel(numeric(dx, 'dx'), dy === undefined ? 0 : numeric(dy, 'dy'))
}
