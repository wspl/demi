// `bun browse click <locator|x,y>`: clicks the one element a locator names,
// or a point of the viewport, as a person does with the mouse.
import { CommandFailure, parse, type Context } from '../command'
import { resolve, uncovered } from '../find'

const USAGE = 'click <locator|x,y> [--right | --middle] [--double] [--modifiers Meta,Shift]'
const OPTIONS = {
  right: { type: 'boolean' },
  middle: { type: 'boolean' },
  double: { type: 'boolean' },
  modifiers: { type: 'string' },
} as const

type Modifier = 'Alt' | 'Control' | 'ControlOrMeta' | 'Meta' | 'Shift'
const MODIFIERS: readonly Modifier[] = ['Alt', 'Control', 'ControlOrMeta', 'Meta', 'Shift']

function modifiers(text: string | undefined): Modifier[] {
  if (text === undefined) {
    return []
  }
  return text.split(',').map((name) => {
    const modifier = MODIFIERS.find((known) => known === name.trim())
    if (!modifier) {
      throw new CommandFailure(`${name} is not a modifier; use ${MODIFIERS.join(', ')}`)
    }
    return modifier
  })
}

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  if (positionals.length !== 1) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const button = values.right ? 'right' : values.middle ? 'middle' : 'left'
  const clickCount = values.double ? 2 : 1
  const held = modifiers(values.modifiers)
  const target = await resolve(context, positionals[0])
  const page = await context.browser.page()
  if (target.kind === 'point') {
    for (const key of held) {
      await page.keyboard.down(key)
    }
    await page.mouse.click(target.x, target.y, { button, clickCount })
    for (const key of held.toReversed()) {
      await page.keyboard.up(key)
    }
    return
  }
  await uncovered(context, target.locator, target.text)
  await target.locator.click({ button, clickCount, modifiers: held, timeout: 10_000 })
}
