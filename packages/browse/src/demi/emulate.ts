// `demi.emulate({ viewport, scale, theme, device, locale, timeZone })`: sets
// the browser's viewport, pixel ratio, theme, a phone Playwright knows
// (`device: 'iPhone 15'`), locale or time zone, for the pages as they are,
// without reloading them; what it leaves out stays as it was, and `reset`
// starts from the defaults. The slot's state keeps them for every later page
// and browser. In the gallery the theme also sets the gallery's own theme,
// which it otherwise keeps whatever the system's is. Answers the screen the
// page now has.
import { screenOf, type Screen } from '../emulation'
import { readState, updateState, type Emulation } from '../state'
import { Failure, type Tool } from '../tool'
import { applyGalleryTheme } from './gallery'

export interface EmulateOptions {
  viewport?: { width: number, height: number }
  scale?: number
  theme?: 'light' | 'dark'
  device?: string
  locale?: string
  timeZone?: string
  reset?: boolean
}

export async function emulate(tool: Tool, options: EmulateOptions): Promise<Screen> {
  const before = readState(tool.slot).emulation ?? {}
  const change: Emulation = options.reset ? {} : { ...before }
  // A phone brings its own screen: a size or a scale set before it would
  // otherwise stay and give the phone a desktop's viewport.
  if (options.device !== undefined) {
    delete change.width
    delete change.height
    delete change.scale
  }
  if (options.viewport !== undefined) {
    change.width = options.viewport.width
    change.height = options.viewport.height
  }
  change.scale = options.scale ?? change.scale
  change.theme = options.theme ?? change.theme
  change.device = options.device ?? change.device
  change.locale = options.locale ?? change.locale
  change.timezone = options.timeZone ?? change.timezone
  let screen: Screen
  try {
    screen = screenOf(change)
  } catch (error) {
    // A device Playwright does not know fails here, before anything changes.
    throw new Failure(error instanceof Error ? error.message : String(error))
  }
  updateState(tool.slot, (state) => {
    state.emulation = change
  })
  await tool.browser.emulate(change)
  const page = await tool.browser.existingPage()
  if (page && change.theme !== undefined) {
    await applyGalleryTheme(tool.slot, page, change.theme)
  }
  return screen
}
