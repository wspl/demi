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
  /** Whether the network policy decides what its Clouds reach. */
  networkPolicy: boolean
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

/** What the server's Clouds reach (`crates/web-api-protocol/src/settings.rs`). */
export type NetworkPolicy = 'noNetwork' | 'developerServices' | 'internetOnly' | 'internetAndPrivateNetwork' | 'custom'

/** The common policies in the order the menu offers them; Custom follows them, apart. */
export const COMMON_NETWORK_POLICIES: readonly NetworkPolicy[] = [
  'noNetwork',
  'developerServices',
  'internetOnly',
  'internetAndPrivateNetwork',
]

/** A group of destinations a policy allows whole. */
export type NetworkGroup = 'internet' | 'privateNetwork' | 'developerServices'

/** The groups in the order Custom lists them. */
export const NETWORK_GROUPS: readonly NetworkGroup[] = ['internet', 'privateNetwork', 'developerServices']

/** What Custom allows: the groups switched on, and the domains and addresses as the master wrote them. */
export interface CustomNetworkPolicy extends Record<NetworkGroup, boolean> {
  allowed: string[]
}

/**
 * Settings › Cloud's network policy. Custom's switches and list are kept
 * while a common policy is chosen; the list applies only under Custom.
 */
export interface CloudNetworkSetting {
  policy: NetworkPolicy
  custom: CustomNetworkPolicy
  /** The domains of Developer Services, which the server ships. */
  developerServices: string[]
}

/**
 * `setting` with `policy` chosen. Choosing Custom from a common policy sets
 * its switches to what that policy allows, so the master starts from it;
 * Custom's list is kept either way.
 */
export function withPolicy(setting: CloudNetworkSetting, policy: NetworkPolicy): CloudNetworkSetting {
  const from = setting.policy
  if (policy !== 'custom' || from === 'custom') {
    return { ...setting, policy }
  }
  const custom = {
    ...setting.custom,
    internet: from === 'internetOnly' || from === 'internetAndPrivateNetwork',
    privateNetwork: from === 'internetAndPrivateNetwork',
    developerServices: from === 'developerServices',
  }
  return { ...setting, policy, custom }
}

/** Why the server refused a domain or an address, the `problem` its refusal names. */
export type DestinationProblem = 'entry' | 'port' | 'neverReachable'

/** What a row says under a destination the server refused for `problem`. */
export function destinationProblemText(problem: DestinationProblem): SentenceText {
  return t(`cloud.problems.${problem}`)
}
