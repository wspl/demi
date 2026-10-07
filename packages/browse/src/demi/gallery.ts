// `demi.gallery(path)` (browse.md § What `demi` adds): opens a specimen page
// of the slot's gallery, such as `/session?view=blocks`, with the theme
// `demi.emulate` set.
//
// The gallery keeps a theme of its own, light or dark whatever the system's
// is, so a theme `demi.emulate` sets reaches it only through the gallery's
// state. The gallery's development server serves that state as a module,
// and the page's module map holds the instance the gallery runs, so
// importing it by its address gives the tool that instance; setting its mode
// changes the theme as the gallery's own menu does, without a reload.
import type { Page, Response } from 'playwright'
import { local, type Slot } from '../slot'
import { readState } from '../state'
import { Failure, type Tool } from '../tool'

/** The address the gallery's development server serves its state module at. */
const STATE_MODULE = '/src/gallery-state.ts'

export async function gallery(tool: Tool, path = '/'): Promise<Response | null> {
  const page = await tool.browser.page()
  const response = await page.goto(local(tool.slot.ports.gallery) + (path.startsWith('/') ? path : `/${path}`))
  const theme = readState(tool.slot).emulation?.theme
  if (theme !== undefined) {
    await applyGalleryTheme(tool.slot, page, theme)
  }
  return response
}

/**
 * Sets the gallery's theme when `page` shows the gallery; answers whether
 * it did.
 */
export async function applyGalleryTheme(slot: Slot, page: Page, theme: 'light' | 'dark'): Promise<boolean> {
  if (new URL(page.url()).origin !== local(slot.ports.gallery)) {
    return false
  }
  const shown = await page.evaluate(async ({ module, mode }) => {
    const state: unknown = await import(module)
    if (typeof state !== 'object' || state === null || !('galleryState' in state)) {
      return null
    }
    const gallery = state.galleryState
    if (typeof gallery !== 'object' || gallery === null || !('mode' in gallery)) {
      return null
    }
    gallery.mode = mode
    // The gallery writes its theme onto the document once Vue flushes the change.
    await new Promise((resolve) => requestAnimationFrame(resolve))
    return document.documentElement.getAttribute('data-theme')
  }, { module: STATE_MODULE, mode: theme })
  if (shown !== theme) {
    throw new Failure(`The gallery's theme did not change to ${theme}: ${STATE_MODULE} ${shown === null ? 'has no galleryState with a mode' : `left the page's theme ${shown}`}`)
  }
  return true
}
