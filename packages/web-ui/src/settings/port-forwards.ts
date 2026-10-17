import { reasonSentence } from '../devices/direct'
import type { DeviceState } from '../devices/state'
import type { SentenceText } from '../ui/ui-text'
import { t } from './locales/text'

/**
 * Why a forward's connections go through the relay after a direct path
 * between its two runners failed: the direct channel's own reasons
 * (`port-forwarding.md` § What the user sees).
 */
export type ForwardPairReason =
  | { kind: 'unreachable' }
  | { kind: 'network'; side: 'browser' | 'device' }
  | { kind: 'busy' }
  | { kind: 'dropped' }
  | { kind: 'notOffered' }

/** What the Listen On runner last announced of a forward. */
export type ForwardRunState =
  | { kind: 'listening'; direct: number; relay: number; reason?: ForwardPairReason | null }
  | { kind: 'portInUse' }

/** A forward as Settings › Port Forwarding shows and edits it. */
export interface SettingsPortForward {
  id: string
  name: string
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
  Pick<SettingsPortForward, 'name' | 'listenDevice' | 'listenPort' | 'connectDevice' | 'connectAddress' | 'enabled'>
>

/** One of the user's Hosts, as a forward's ends are chosen among them: the Cloud is named Cloud. */
export interface ForwardHost {
  id: string
  name: string
  state: DeviceState
}

function hostName(hosts: readonly ForwardHost[], id: string): string {
  return hosts.find((host) => host.id === id)?.name ?? t('portForwards.unknownHost')
}

/** The forward's ends in one line: *Cloud · 11434 → MacBook Pro · localhost:11434*. */
export function forwardRoute(forward: SettingsPortForward, hosts: readonly ForwardHost[]): string {
  const listen = `${hostName(hosts, forward.listenDevice)} · ${forward.listenPort}`
  const connect = forward.connectDevice === null
    ? forward.connectAddress
    : `${hostName(hosts, forward.connectDevice)} · ${forward.connectAddress}`
  return `${listen} → ${connect}`
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
