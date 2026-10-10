import { inject, provide, type InjectionKey } from 'vue'
import type { WaitingCall } from '@demicodes/protocol'
import type { TerminalRecord } from './terminals'

/**
 * The command a `shell` call started, by the call's tool-use id, in the
 * transcript a list shows: while the call runs, its live output shows under
 * it, and while the command runs, also after the call returned, its row
 * shimmers (`runtime.md` § Rendering boundary).
 */
export type LiveCallLookup = (toolUseId: string) => TerminalRecord | undefined

const liveCallsKey: InjectionKey<LiveCallLookup> = Symbol('live-calls')

/** Gives the transcript below its calls' commands; a subagent's panel gives its own. */
export function provideLiveCalls(lookup: LiveCallLookup): void {
  provide(liveCallsKey, lookup)
}

export function useLiveCalls(): LiveCallLookup {
  return inject(liveCallsKey, () => undefined)
}

/**
 * The Host a `shell` call waits for, by the call's tool-use id, while its
 * runner is away: the call's row reads *Waiting for* it
 * (`sessions-and-targets.md` § Host operations).
 */
export type CallWaitLookup = (toolUseId: string) => string | undefined

const callWaitsKey: InjectionKey<CallWaitLookup> = Symbol('call-waits')

/** Gives the transcript below the Hosts its calls wait for; a subagent's panel gives its own. */
export function provideCallWaits(lookup: CallWaitLookup): void {
  provide(callWaitsKey, lookup)
}

export function useCallWaits(): CallWaitLookup {
  return inject(callWaitsKey, () => undefined)
}

/** The Host the call `toolUseId` among `waiting` waits for. */
export function waitedHost(waiting: readonly WaitingCall[] | undefined, toolUseId: string): string | undefined {
  return waiting?.find((call) => call.toolUseId === toolUseId)?.host
}
