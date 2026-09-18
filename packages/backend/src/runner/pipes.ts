// The pipes of a relayed `rpc` call (`runner.md` § Pipes). The broker itself
// is `@demicodes/host-remote`'s: the Host's file contents travel through it
// as well.
import type { Pipe } from '@demicodes/host-remote'
import type { CommandIO } from '@demicodes/shell'

/**
 * The pipes of a relayed `rpc` call, carried on the handler's `io` so a
 * handler that attaches them to a job elsewhere (`demi host shell`) can
 * name the far ends instead of copying bytes through this process.
 */
export interface RelayedPipes {
  stdin: Pipe | null
  stdout: Pipe
}

const RELAYED_PIPES = Symbol('relayedPipes')

export function withRelayedPipes(
  io: CommandIO,
  pipes: RelayedPipes
): CommandIO {
  return Object.assign(io, { [RELAYED_PIPES]: pipes })
}

export function relayedPipesOf(io: CommandIO): RelayedPipes | null {
  return (io as CommandIO & { [RELAYED_PIPES]?: RelayedPipes })[RELAYED_PIPES] ??
    null
}
