// `bun check emulate …`: sets the browser's viewport, pixel ratio, theme, a
// phone Playwright knows (`--device "iPhone 15"`), locale or time zone. The
// slot's state keeps them for every later browser; a change of the pixel
// ratio, phone, locale or time zone starts the browser again on the same
// address, since a context takes those only when it starts.
import { CheckFailure, numeric, parse, type Context } from '../command'
import { launchOptions } from '../browser'
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
    throw new CheckFailure(`Usage: bun check ${USAGE}`)
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
      throw new CheckFailure(`--size takes WIDTHxHEIGHT, such as 1440x900, not ${values.size}`)
    }
    change.width = Number(size[1])
    change.height = Number(size[2])
  }
  if (values.scale !== undefined) {
    change.scale = numeric(values.scale, '--scale')
  }
  if (values.theme !== undefined) {
    if (values.theme !== 'light' && values.theme !== 'dark') {
      throw new CheckFailure(`--theme is light or dark, not ${values.theme}`)
    }
    change.theme = values.theme
  }
  change.device = values.device ?? change.device
  change.locale = values.locale ?? change.locale
  change.timezone = values.timezone ?? change.timezone
  // A device Playwright does not know fails here, before anything changes.
  const options = launchOptions(change)
  updateState(context.slot, (state) => {
    state.emulation = change
  })
  const restart = (['scale', 'device', 'locale', 'timezone'] as const).some((key) => change[key] !== before[key])
  const page = await context.browser.existingPage()
  if (page && restart) {
    await context.browser.relaunch()
  } else if (page) {
    await page.setViewportSize(options.viewport)
    await page.emulateMedia({ colorScheme: change.theme ?? null })
  }
  const parts = [
    `${options.viewport.width}×${options.viewport.height} at ${options.deviceScaleFactor}x`,
    change.theme ? `${change.theme} theme` : 'the system theme',
    change.device,
    change.locale,
    change.timezone,
  ].filter((part) => part !== undefined)
  context.print(parts.join(', ') + (page && restart ? ' (the browser started again)' : ''))
}
