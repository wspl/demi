// `bun check open <address>`: opens an address of the web app (`/chat/c-1`),
// of the gallery (`gallery:/session?view=blocks`), or any URL, and waits
// until the page has loaded.
import { addressUrl, CheckFailure, parse, timeoutMs, timeoutOption, type Context } from '../command'

const USAGE = 'open <address> [--timeout <s>]'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, timeoutOption, USAGE)
  const [address] = positionals
  if (positionals.length !== 1 || address === undefined) {
    throw new CheckFailure(`Usage: bun check ${USAGE}`)
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
}
