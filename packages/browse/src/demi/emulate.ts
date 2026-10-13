// `demi.emulate({ viewport, scale, theme, device, locale, timeZone })`: sets
// the browser's viewport, pixel ratio, theme, a phone Playwright knows
// (`device: 'iPhone 15'`), locale or time zone, for the pages as they are,
// without reloading them; what it leaves out stays as it was, and `reset`
// starts from the defaults. The slot's state keeps them for every later page
// and browser. In the gallery the theme also sets the gallery's own theme,
// which it otherwise keeps whatever the system's is. Answers the screen the
// page now has.
//
// The tool is the one owner of these settings (browse.md § Conditions): on
// every page of the slot's context, Playwright's `setViewportSize` and
// `emulateMedia({ colorScheme })` are `demi.emulate` under Playwright's
// names (`routeToEmulate`), since Playwright's own session setting them
// would be undone by the tool's at the next load.
import type { Page } from 'playwright'
import { screenOf, type Screen } from '../emulation'
import { readState, updateState, type Emulation } from '../state'
import { Failure, type Tool } from '../tool'
import { applyGalleryTheme } from './gallery'

/** What `demi.emulate` needs of the call: the slot and its browser. */
type Owner = Pick<Tool, 'slot' | 'browser'>

export interface EmulateOptions {
  viewport?: { width: number, height: number }
  scale?: number
  theme?: 'light' | 'dark'
  device?: string
  locale?: string
  timeZone?: string
  reset?: boolean
}

export async function emulate(tool: Owner, options: EmulateOptions): Promise<Screen> {
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
  const pages = await tool.browser.emulate(change)
  if (change.theme !== undefined) {
    for (const page of pages) {
      await applyGalleryTheme(tool.slot, page, change.theme)
    }
  }
  return screen
}

/**
 * Makes `page`'s `setViewportSize`, `viewportSize` and the colour scheme of
 * its `emulateMedia` the slot's emulation. The media `emulateMedia` sets
 * besides, such as reduced motion, stay Playwright's: the tool sets no other,
 * and applying the slot's emulation again after it keeps the theme the tool's.
 */
export function routeToEmulate(page: Page, owner: Owner): void {
  const playwrightMedia = page.emulateMedia.bind(page)
  page.setViewportSize = async (viewport) => {
    await emulate(owner, { viewport })
  }
  page.viewportSize = () => {
    const { width, height } = screenOf(readState(owner.slot).emulation ?? {})
    return { width, height }
  }
  page.emulateMedia = async (options = {}) => {
    const { colorScheme, ...rest } = options
    if (colorScheme === null || colorScheme === 'no-preference') {
      throw new Failure(`The slot's theme is light or dark, not ${colorScheme}: demi.emulate({ reset: true }) goes back to the system's`)
    }
    if (Object.keys(rest).length > 0) {
      await playwrightMedia(rest)
    }
    await emulate(owner, { theme: colorScheme })
  }
}
