import type { SentenceText } from '../ui/ui-text'
import { t } from './locales/text'

/** How the server's Clouds are isolated (`crates/web-api-protocol/src/settings.rs`). */
export type Isolation = 'simple' | 'gvisor'

/** The isolations in the order the menu offers them. */
export const ISOLATIONS: readonly Isolation[] = ['simple', 'gvisor']

/** What the server can do with one isolation. */
export interface IsolationRuntime {
  isolation: Isolation
  /** Why the server can't run it, in its own words; null when it can. */
  unavailable: string | null
  /** Whether Private Network Access decides what its Clouds reach. */
  privateNetwork: boolean
  /** The download of its release, which the server makes when the master chooses it; null when it runs without one. */
  download: IsolationDownload | null
}

/** Where the server is with the download of an isolation's release. */
export type IsolationDownload =
  | { state: 'waiting' }
  /** `received` bytes of `total`, which the source may not say. */
  | { state: 'downloading', received: number, total: number | null }
  /** Downloaded; the server's machine manager restarts to run it. */
  | { state: 'starting' }
  /** The last download failed; choosing the isolation again retries it. */
  | { state: 'failed', message: string }

/**
 * Settings › Cloud's isolation: the one chosen, and what the server can do
 * with each, null while its machine manager has not been reached.
 */
export interface CloudIsolationSetting {
  isolation: Isolation
  runtimes: IsolationRuntime[] | null
}

/** Which private destinations the server's Clouds reach (`crates/web-api-protocol/src/settings.rs`). */
export type PrivateNetworkAccess = 'off' | 'entire' | 'only'

/** Settings › Cloud's Private Network Access: the access, and the destinations Only These Addresses allows, as the master wrote them. */
export interface CloudNetworkSetting {
  access: PrivateNetworkAccess
  destinations: string[]
}

/** The accesses in the order the menu offers them. */
export const PRIVATE_NETWORK_ACCESSES: readonly PrivateNetworkAccess[] = ['off', 'entire', 'only']

/** Why the server refused a destination, the `problem` its refusal names. */
export type DestinationProblem = 'address' | 'port' | 'neverReachable'

/** What a row says under a destination the server refused for `problem`. */
export function destinationProblemText(problem: DestinationProblem): SentenceText {
  return t(`cloud.problems.${problem}`)
}
