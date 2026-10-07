// `bun check net …`: the conditions of the page's network (product-checks.md
// § Conditions), for every request and socket between the page and the web
// app: offline or online, a round trip's latency, a bandwidth limit, or a
// cut of the connections whose requests match a pattern, which closes their
// sockets without a close frame, as a lost connection does.
import { CheckFailure, numeric, parse, type Context } from '../command'

const USAGE = 'net [offline | online | latency <ms> | bandwidth <kbit/s>|off | cut <pattern> | reset]'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  const [action, argument] = positionals
  const network = await context.network()
  const browserContext = async () => (await context.browser.page()).context()
  switch (action) {
    case undefined:
      break
    case 'offline':
      network.set({ offline: true })
      // The page's own view: navigator.onLine and requests to other servers.
      await (await browserContext()).setOffline(true)
      break
    case 'online':
      network.set({ offline: false })
      await (await browserContext()).setOffline(false)
      break
    case 'latency':
      network.set({ latencyMs: numeric(needs(argument), 'The latency') })
      break
    case 'bandwidth': {
      const value = needs(argument)
      network.set({ kbps: value === 'off' ? null : numeric(value, 'The bandwidth') })
      break
    }
    case 'cut': {
      const cut = network.cut(needs(argument))
      if (cut.length === 0) {
        throw new CheckFailure([`No connection of the page matches ${argument}. Open now:`, ...network.list().map((line) => `  ${line}`)].join('\n'))
      }
      context.print(`Cut ${cut.length}:`)
      for (const line of cut) {
        context.print(`  ${line}`)
      }
      return
    }
    case 'reset':
      network.set({ offline: false, latencyMs: 0, kbps: null })
      await (await browserContext()).setOffline(false)
      break
    default:
      throw new CheckFailure(`Usage: bun check ${USAGE}`)
  }
  const state = network.state
  const conditions = [
    state.offline ? 'offline' : 'online',
    `latency ${state.latencyMs} ms`,
    state.kbps === null ? 'no bandwidth limit' : `${state.kbps} kbit/s`,
  ]
  context.print(conditions.join(', '))
  if (action === undefined) {
    for (const line of network.list()) {
      context.print(`  ${line}`)
    }
  }

  function needs(value: string | undefined): string {
    if (value === undefined) {
      throw new CheckFailure(`Usage: bun check ${USAGE}`)
    }
    return value
  }
}
