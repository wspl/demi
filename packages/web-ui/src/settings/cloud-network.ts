import type { SentenceText } from '../ui/ui-text'
import { t } from './locales/text'

/** Which private destinations the server's Clouds reach (`crates/web-api-protocol/src/settings.rs`). */
export type PrivateNetworkAccess = 'off' | 'entire' | 'only'

/** Settings › Cloud Network: the access, and the destinations Only These Addresses allows, as the master wrote them. */
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
  return t(`cloudNetwork.problems.${problem}`)
}
