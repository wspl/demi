// What every `demi` helper gets from the tool's server for the call it runs
// in (browse.md § What `demi` adds), and how a helper fails.
import type { Browser } from './browser'
import type { Network } from './network'
import { local, type Slot } from './slot'

/** A screenshot the call wrote, which the call's report lists. */
export interface Shot {
  /** What wrote it: `shot`, `timeline` or `failure`. */
  kind: string
  path: string
  /** What the report says of it, such as its size or the moment it shows. */
  detail: string
}

export interface Tool {
  slot: Slot
  browser: Browser
  /** The slot's network, which the browser reaches the web app through. */
  network(): Promise<Network>
  /** Prints a line of the call's output on the caller's terminal. */
  print(line: string): void
  /** The caller's `DEMI_*` variables, which the processes the tool starts get. */
  env: Record<string, string>
  /** Records a screenshot the call wrote, for its report. */
  wrote(shot: Shot): void
  /** Runs `release` when the call ends, however it ends, for what a helper opened only for the call. */
  release(release: () => Promise<void>): void
  /** Ends the server once the call has answered, as `demi.stop` and `demi.down` do. */
  endServer(): void
}

/** A failure a helper explains itself: its message is all the caller needs, never the tool's stack. */
export class Failure extends Error {}

/** The base the browser opens the web app at: the slot's network. */
export function webBase(slot: Slot): string {
  return local(slot.ports.network)
}
