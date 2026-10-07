// `bun browse net …`: the conditions of the page's network (browse.md
// § Conditions), for every request and socket between the page and the web
// app: offline or online, a round trip's latency, a bandwidth limit, or a
// cut of the connections whose requests match a pattern, which closes their
// sockets without a close frame, as a lost connection does. `unreachable`
// holds the page's traffic and refuses its new connections while the browser
// still reports a network, as when the way to a far server is lost: the page
// sees its sockets go quiet and its reconnects fail, not that it is offline.
import { CommandFailure, numeric, parse, type Context } from '../command'
import { NORMAL } from '../network'
import { updateState, type Conditions } from '../state'

const USAGE = 'net [offline | unreachable | online | latency <ms> | bandwidth <kbit/s>|off | cut <pattern> | reset]'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  const [action, argument] = positionals
  const network = await context.network()
  let change: Partial<Conditions>
  switch (action) {
    case undefined:
      change = {}
      break
    case 'offline':
      change = { reach: 'offline' }
      break
    case 'unreachable':
      change = { reach: 'unreachable' }
      break
    case 'online':
      change = { reach: 'online' }
      break
    case 'latency':
      change = { latencyMs: numeric(needs(argument), 'The latency') }
      break
    case 'bandwidth': {
      const value = needs(argument)
      change = { kbps: value === 'off' ? null : numeric(value, 'The bandwidth') }
      break
    }
    case 'cut': {
      const cut = network.cut(needs(argument))
      if (cut.length === 0) {
        throw new CommandFailure([`No connection of the page matches ${argument}. Open now:`, ...network.list().map((line) => `  ${line}`)].join('\n'))
      }
      context.print(`Cut ${cut.length}:`)
      for (const line of cut) {
        context.print(`  ${line}`)
      }
      return
    }
    case 'reset':
      change = NORMAL
      break
    default:
      throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  network.set(change)
  const state = network.state
  // The slot's state keeps them for a daemon that starts again with the same browser.
  updateState(context.slot, (slot) => {
    slot.net = { ...state }
  })
  if (change.reach !== undefined) {
    // The page's own view, navigator.onLine and requests to other servers, is offline only when `offline`.
    await (await context.browser.page()).context().setOffline(state.reach === 'offline')
  }
  const conditions = [
    state.reach,
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
      throw new CommandFailure(`Usage: bun browse ${USAGE}`)
    }
    return value
  }
}
