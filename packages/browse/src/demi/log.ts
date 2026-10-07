// `demi.log.console()`, `.network()` and `.sockets()` (browse.md § What
// `demi` adds): what the page logged, requested, or sent and received on its
// sockets and direct channels since the last `demi.log.mark(name)`, or since
// the tool attached to the browser with `{ all: true }`, one line each.
// `network` leaves out the dev server's modules, styles and fonts unless
// `{ modules: true }`.
import type { LogKind } from '../logs'
import { Failure, type Tool } from '../tool'

export interface LogOptions {
  all?: boolean
  modules?: boolean
}

export function log(tool: Tool) {
  async function read(kind: LogKind, options: LogOptions = {}): Promise<string[]> {
    if (!await tool.browser.attachRunning()) {
      throw new Failure('The tool is attached to no browser in this slot, so it logged nothing')
    }
    return tool.browser.logs.read(kind, { all: options.all ?? false, modules: options.modules ?? false })
  }

  return {
    console: (options?: LogOptions) => read('console', options),
    network: (options?: LogOptions) => read('network', options),
    sockets: (options?: LogOptions) => read('sockets', options),
    /** Starts the logs over from now; answers the mark's name. */
    mark: (name?: string) => tool.browser.logs.mark(name ?? null),
  }
}
