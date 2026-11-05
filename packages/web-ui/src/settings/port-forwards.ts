import { reasonSentence } from '../devices/direct'
import type { DeviceMark } from '../devices/mark'
import type { DeviceState } from '../devices/state'
import type { HostKind } from '../hosts/types'
import type { SentenceText } from '../ui/ui-text'
import { t } from './locales/text'

/**
 * Why a forward's connections go through the relay after a direct path
 * between its two runners failed, or measured slower: the direct channel's
 * own reasons (`crates/runner-protocol/src/forward.rs`).
 */
export type ForwardPairReason =
  | { kind: 'unreachable' }
  | { kind: 'network'; side: 'browser' | 'device' }
  | { kind: 'busy' }
  | { kind: 'slower' }
  | { kind: 'dropped' }
  | { kind: 'notOffered' }

/** What the Listen On runner last announced of a forward. */
export type ForwardRunState =
  | { kind: 'listening'; direct: number; relay: number; reason?: ForwardPairReason | null }
  | { kind: 'portInUse' }

/** A forward as Settings › Port Forwarding shows and edits it: its route, which nothing names. */
export interface SettingsPortForward {
  id: string
  listenDevice: string
  listenPort: number
  /** Null until the user chooses the Host it connects on. */
  connectDevice: string | null
  connectAddress: string
  enabled: boolean
  /** What its Listen On runner last announced; null while it has said nothing. */
  state: ForwardRunState | null
}

/** What one edit of a forward's page changes. */
export type PortForwardChange = Partial<
  Pick<SettingsPortForward, 'listenDevice' | 'listenPort' | 'connectDevice' | 'connectAddress' | 'enabled'>
>

/** One of the user's Hosts, as a forward's ends are chosen among them: the Cloud is named Cloud. */
export interface ForwardHost {
  id: string
  name: string
  kind: HostKind
  state: DeviceState
  /** A device's mark, with the number it shows (`shownMark`); null for the Cloud. */
  mark: DeviceMark | null
}

function hostName(hosts: readonly ForwardHost[], id: string): string {
  return hosts.find((host) => host.id === id)?.name ?? t('portForwards.unknownHost')
}

/** Where the forward listens: *Cloud · 11434*. */
export function forwardListen(forward: SettingsPortForward, hosts: readonly ForwardHost[]): string {
  return `${hostName(hosts, forward.listenDevice)} · ${forward.listenPort}`
}

/** Where the forward connects: *MacBook Pro · localhost:11434*, the address alone while no Host is chosen. */
export function forwardConnect(forward: SettingsPortForward, hosts: readonly ForwardHost[]): string {
  return forward.connectDevice === null
    ? forward.connectAddress
    : `${hostName(hosts, forward.connectDevice)} · ${forward.connectAddress}`
}

/** The forward's ends in one line, which says the forward wherever it is said alone: *Cloud · 11434 → MacBook Pro · localhost:11434*. */
export function forwardRoute(forward: SettingsPortForward, hosts: readonly ForwardHost[]): string {
  return `${forwardListen(forward, hosts)} → ${forwardConnect(forward, hosts)}`
}

/**
 * What a forward does now, one row of the design's table: off, its Connect
 * To missing, a Host offline, which the page knows from the user's devices,
 * or what its Listen On runner announced.
 */
export function forwardState(forward: SettingsPortForward, hosts: readonly ForwardHost[]): SentenceText {
  if (!forward.enabled) {
    return t('portForwards.state.off')
  }
  if (forward.connectDevice === null) {
    return t('portForwards.state.chooseHost')
  }
  for (const id of [forward.listenDevice, forward.connectDevice]) {
    const host = hosts.find((candidate) => candidate.id === id)
    if (host?.state !== 'online') {
      return t('portForwards.state.offline', { host: hostName(hosts, id) })
    }
  }
  const state = forward.state
  if (state === null) {
    return t('portForwards.state.starting')
  }
  if (state.kind === 'portInUse') {
    return t('portForwards.state.portInUse', { port: forward.listenPort, host: hostName(hosts, forward.listenDevice) })
  }
  const paths = [
    ...(state.direct > 0 ? [t('portForwards.state.direct', { count: state.direct }, state.direct)] : []),
    ...(state.relay > 0 ? [t('portForwards.state.relay', { count: state.relay }, state.relay)] : []),
  ]
  return paths.length ? paths.join(t('portForwards.state.and')) : t('portForwards.state.idle')
}

/**
 * How a forward fares, for its dot: off; failed, its port taken; waiting,
 * its Connect To missing or a Host offline; carrying connections; or idle,
 * listening with none or starting.
 */
export type ForwardTone = 'off' | 'failed' | 'waiting' | 'carrying' | 'idle'

export function forwardTone(forward: SettingsPortForward, hosts: readonly ForwardHost[]): ForwardTone {
  if (!forward.enabled) {
    return 'off'
  }
  const away = [forward.listenDevice, forward.connectDevice].some(
    (id) => id === null || hosts.find((host) => host.id === id)?.state !== 'online',
  )
  if (away) {
    return 'waiting'
  }
  const state = forward.state
  if (state?.kind === 'portInUse') {
    return 'failed'
  }
  return state?.kind === 'listening' && state.direct + state.relay > 0 ? 'carrying' : 'idle'
}

/**
 * Why the forward's connections go through the relay, in the direct
 * channel's words; null while nothing failed or the forward does not listen.
 */
export function forwardReason(forward: SettingsPortForward): SentenceText | null {
  const state = forward.state
  if (!forward.enabled || state?.kind !== 'listening' || !state.reason) {
    return null
  }
  return reasonSentence(state.reason)
}
