// `demi.hide({ page })` and `demi.show({ page })`: hide the slot's page, or
// `page`, as a tab another one covers, and bring it back, for a check of what
// the page does while hidden. The page sees `document.visibilityState` change
// and gets `visibilitychange` each time. A page a call leaves hidden is shown
// again when the call ends.
//
// Bringing another page to the front does not hide a page: the tool emulates
// focus on every page, which keeps it visible (`browser.ts`).
import type { Page } from 'playwright'
import type { Tool } from '../tool'

export interface VisibilityOptions {
  page?: Page
}

export async function hide(tool: Tool, options: VisibilityOptions = {}): Promise<void> {
  const page = await pageOf(tool, options)
  tool.release(() => tool.browser.show(page))
  await tool.browser.hide(page)
}

export async function show(tool: Tool, options: VisibilityOptions = {}): Promise<void> {
  await tool.browser.show(await pageOf(tool, options))
}

/** The page itself: the script's `page` stands for the slot's (call.ts, `addressed`), and its main frame names the page. */
async function pageOf(tool: Tool, options: VisibilityOptions): Promise<Page> {
  return options.page?.mainFrame().page() ?? await tool.browser.page()
}
