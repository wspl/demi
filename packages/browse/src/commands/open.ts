// `bun browse open <address>`: opens an address of the web app (`/chat/c-1`),
// of the gallery (`gallery:/session?view=blocks`), or any URL, and waits
// until the page has loaded. A gallery page gets the theme `emulate` set.
import { addressUrl, CommandFailure, parse, timeoutMs, timeoutOption, type Context } from '../command'
import { applyGalleryTheme } from '../gallery'
import { readState } from '../state'

const USAGE = 'open <address> [--timeout <s>]'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, timeoutOption, USAGE)
  const [address] = positionals
  if (positionals.length !== 1 || address === undefined) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const url = addressUrl(context.slot, address)
  if (url.startsWith(`http://127.0.0.1:${context.slot.ports.network}`)) {
    // The browser reaches the web app through the slot's network.
    await context.network()
  }
  const page = await context.browser.page()
  const response = await page.goto(url, { timeout: timeoutMs(values.timeout, 30) })
  const status = response ? ` (${response.status()})` : ''
  context.print(`${page.url()}${status}`)
  const theme = readState(context.slot).emulation?.theme
  if (theme !== undefined && await applyGalleryTheme(context.slot, page, theme)) {
    context.print(`The gallery's own theme is ${theme}, as emulate set`)
  }
}
