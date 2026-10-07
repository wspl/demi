// `demi.net` (browse.md § What `demi` adds): the conditions of the page's
// network, for every request and socket between the page and the web app:
// a round trip's latency, a bandwidth limit, offline or unreachable, or a
// cut of the connections whose requests match a pattern, which closes their
// sockets without a close frame, as a lost connection does. `unreachable`
// holds the page's traffic and refuses its new connections while the
// browser still reports a network, as when the way to a far server is lost:
// the page sees its sockets go quiet and its reconnects fail, not that it is
// offline. Each answers the conditions as they now are.
import { NORMAL } from '../network'
import { updateState, type Conditions } from '../state'
import { Failure, type Tool } from '../tool'

export function net(tool: Tool) {
  /** Changes the conditions, which the slot's state keeps for a server that starts again with the same browser. */
  async function set(change: Partial<Conditions>): Promise<Conditions> {
    const network = await tool.network()
    network.set(change)
    const state = { ...network.state }
    updateState(tool.slot, (slot) => {
      slot.net = state
    })
    if (change.reach !== undefined) {
      // The page's own view, navigator.onLine and requests to other servers, is offline only when `offline`.
      await (await tool.browser.page()).context().setOffline(state.reach === 'offline')
    }
    return state
  }

  return {
    latency: (ms: number) => set({ latencyMs: ms }),
    /** A limit on each way in kilobits per second, or null for none. */
    bandwidth: (kbps: number | null) => set({ kbps }),
    offline: () => set({ reach: 'offline' }),
    unreachable: () => set({ reach: 'unreachable' }),
    online: () => set({ reach: 'online' }),
    reset: () => set(NORMAL),
    /** Cuts the connections a request of which matches `pattern`, a part of the request line or a regular expression; answers what it cut. */
    cut: async (pattern: string | RegExp): Promise<string[]> => {
      const network = await tool.network()
      const cut = network.cut(pattern)
      if (cut.length === 0) {
        throw new Failure([`No connection of the page matches ${pattern}. Open now:`, ...network.list().map((line) => `  ${line}`)].join('\n'))
      }
      return cut
    },
    /** The conditions, and the page's open connections, one line each. */
    status: async (): Promise<{ conditions: Conditions, connections: string[] }> => {
      const network = await tool.network()
      return { conditions: { ...network.state }, connections: network.list() }
    },
  }
}
