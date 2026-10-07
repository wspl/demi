// `bun browse emulate …`: sets the browser's viewport, pixel ratio, theme, a
// phone Playwright knows (`--device "iPhone 15"`), locale or time zone, for
// the pages as they are, without reloading them. The slot's state keeps them
// for every later page and browser. In the gallery the theme also sets the
// gallery's own theme, which it otherwise keeps whatever the system's is.
import { CommandFailure, numeric, parse, type Context } from '../command'
import { screenOf } from '../emulation'
import { applyGalleryTheme } from '../gallery'
import { readState, updateState, type Emulation } from '../state'

const USAGE = 'emulate [--size WxH] [--scale <n>] [--theme light|dark] [--device <name>] [--locale <id>] [--timezone <id>] [--reset]'
const OPTIONS = {
  size: { type: 'string' },
  scale: { type: 'string' },
  theme: { type: 'string' },
  device: { type: 'string' },
  locale: { type: 'string' },
  timezone: { type: 'string' },
  reset: { type: 'boolean' },
} as const

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  if (positionals.length > 0) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const before = readState(context.slot).emulation ?? {}
  const change: Emulation = values.reset ? {} : { ...before }
  // A phone brings its own screen: a size or a scale set before it would
  // otherwise stay and give the phone a desktop's viewport.
  if (values.device !== undefined) {
    delete change.width
    delete change.height
    delete change.scale
  }
  if (values.size !== undefined) {
    const size = /^(\d+)x(\d+)$/.exec(values.size)
    if (!size) {
      throw new CommandFailure(`--size takes WIDTHxHEIGHT, such as 1440x900, not ${values.size}`)
    }
    change.width = Number(size[1])
    change.height = Number(size[2])
  }
  if (values.scale !== undefined) {
    change.scale = numeric(values.scale, '--scale')
  }
  if (values.theme !== undefined) {
    if (values.theme !== 'light' && values.theme !== 'dark') {
      throw new CommandFailure(`--theme is light or dark, not ${values.theme}`)
    }
    change.theme = values.theme
  }
  change.device = values.device ?? change.device
  change.locale = values.locale ?? change.locale
  change.timezone = values.timezone ?? change.timezone
  let screen
  try {
    screen = screenOf(change)
  } catch (error) {
    // A device Playwright does not know fails here, before anything changes.
    throw new CommandFailure(error instanceof Error ? error.message : String(error))
  }
  updateState(context.slot, (state) => {
    state.emulation = change
  })
  await context.browser.emulate(change)
  const parts = [
    `${screen.width}×${screen.height} at ${screen.scale}x`,
    change.theme ? `${change.theme} theme` : 'the system theme',
    change.device,
    change.locale,
    change.timezone,
  ].filter((part) => part !== undefined)
  context.print(parts.join(', '))
  const page = await context.browser.existingPage()
  if (page && change.theme !== undefined && await applyGalleryTheme(context.slot, page, change.theme)) {
    context.print(`The gallery's own theme is ${change.theme} too`)
  }
}
