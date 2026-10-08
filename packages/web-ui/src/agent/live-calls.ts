import { inject, provide, type InjectionKey } from 'vue'
import type { TerminalRecord } from './terminals'

/**
 * The command a `shell_exec` call started, by the call's tool-use id, in the
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
