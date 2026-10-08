import type { SentenceText, TitleText } from '../ui/ui-text'

/**
 * How this page reaches a paired device, and why not directly
 * (`direct-channel.md` § What the user sees): the device's route, what the
 * page's attempts saw and what it measures of both paths, which the
 * device's page shows, and the reason it gives from them.
 */

/** How pages reach a device: Automatic, Prefer Direct or Server Only. */
export type DeviceRoute = 'automatic' | 'direct' | 'server'

export const DEVICE_ROUTE_LABEL: Record<DeviceRoute, TitleText> = {
  automatic: 'Automatic',
  direct: 'Prefer Direct',
  server: 'Server Only',
}

/** The line under the route that says what the chosen one does. */
export const DEVICE_ROUTE_DESCRIPTION: Record<DeviceRoute, SentenceText> = {
  automatic: 'Uses the faster path.',
  direct: 'Direct whenever it connects.',
  server: 'Never connects directly.',
}

/** Where an attempt ended. */
export type DirectStage = 'permission' | 'gathering' | 'checking' | 'handshake' | 'channel' | 'connected'

export const DIRECT_STAGE_LABEL: Record<DirectStage, SentenceText> = {
  permission: 'Permission',
  gathering: 'Gathering addresses',
  checking: 'Finding a path',
  handshake: 'Encryption handshake',
  channel: 'Opening the channel',
  connected: 'Connected',
}

/** The browser's local network permission, where it reports one. */
export type DirectPermission = 'granted' | 'prompt' | 'denied'

/** The addresses one side offered: on its own networks, and as the internet sees it. */
export interface DirectAddresses {
  local: string[]
  public: string[]
}

/** What one attempt to make a direct channel saw. */
export interface DirectAttempt {
  /** When it began, as an ISO 8601 timestamp. */
  startedAt: string
  /** How long it took to connect or to fail. */
  durationMs: number
  /**
   * `connected` while its channel stands, `failed` when it did not
   * connect, `busy` when the device's runner refused it for having too many
   * connections, `dropped` when it connected and later failed.
   */
  outcome: 'connected' | 'failed' | 'busy' | 'dropped'
  /** Where it ended: `connected` for one that connected. */
  stage: DirectStage | null
  browser: DirectAddresses
  device: DirectAddresses
  /** The address pairs checked, and how many of them answered. */
  pairs: { tried: number; answered: number }
  /** The device's address in use while it was connected, such as `127.0.0.1:60044`. */
  inUse: string | null
  /** The browser's local network permission as the attempt ran. */
  permission: DirectPermission | null
}

/**
 * One path's figures over its last probes (`direct-channel.md` § Measuring
 * the paths), in milliseconds; `loss` is a share from 0 to 1, null for the
 * relay, which runs over TCP and loses nothing.
 */
export interface PathFigures {
  latencyMs: number
  loss: number | null
}

/** What the device's page knows of the paths to it. */
export interface DirectStatus {
  route: DeviceRoute
  /** Whether this server names STUN servers, so a channel may cross networks. */
  crossing: boolean
  permission: DirectPermission | null
  /** Whether this page's peer to it is connected. */
  peer: boolean
  /** Whether the page uses the peer now: connected directly. */
  chosen: boolean
  /** An attempt runs now. */
  trying: boolean
  /** The last attempt; null before the first. */
  attempt: DirectAttempt | null
  /** When the next attempt runs, as an ISO 8601 timestamp; null when none is planned. */
  nextAt: string | null
  /** Each path's figures, null before its first probes are answered. */
  figures: { direct: PathFigures | null; relay: PathFigures | null }
}

/**
 * Why the page reaches a device through the server, one per row of the
 * design's table; `notYet` while no attempt has ended, which has no reason
 * to give.
 */
export type DirectReason =
  | { kind: 'serverOnly' }
  | { kind: 'slower' }
  | { kind: 'blocked' }
  | { kind: 'unreachable' }
  | { kind: 'network'; side: 'browser' | 'device' }
  | { kind: 'busy' }
  | { kind: 'dropped' }
  | { kind: 'notOffered' }
  | { kind: 'notYet' }

/**
 * The reason the page reaches a device through the server, from its route,
 * its peer and what its last attempt saw, in the order the design gives
 * them; null while it reaches it directly.
 */
export function directReason(status: DirectStatus): DirectReason | null {
  if (status.route === 'server') {
    return { kind: 'serverOnly' }
  }
  if (status.chosen) {
    return null
  }
  if (status.peer) {
    return { kind: 'slower' }
  }
  if (status.permission === 'denied') {
    return { kind: 'blocked' }
  }
  const attempt = status.attempt
  if (!attempt || attempt.outcome === 'connected') {
    return { kind: 'notYet' }
  }
  if (attempt.outcome === 'busy') {
    return { kind: 'busy' }
  }
  if (attempt.outcome === 'dropped') {
    return { kind: 'dropped' }
  }
  if (attempt.stage === 'permission') {
    return { kind: 'blocked' }
  }
  if (!status.crossing) {
    return { kind: 'notOffered' }
  }
  if (attempt.browser.public.length === 0) {
    return { kind: 'network', side: 'browser' }
  }
  if (attempt.device.public.length === 0) {
    return { kind: 'network', side: 'device' }
  }
  return { kind: 'unreachable' }
}

/** A figure in whole units, with one decimal only under one: 0.4, 2, 48. */
function wholeUnlessSmall(value: number): string {
  const tenths = Math.round(value * 10) / 10
  return tenths > 0 && tenths < 1 ? String(tenths) : String(Math.round(value))
}

/** A latency as a person reads it: "0.4 ms", "2 ms", "48 ms". */
function formatLatency(ms: number): string {
  return ms < 0.05 ? 'under 0.1 ms' : `${wholeUnlessSmall(ms)} ms`
}

/** A share lost as a person reads it: "0%", "0.5%", "6%". */
function formatLoss(loss: number): string {
  return `${wholeUnlessSmall(loss * 100)}%`
}

/**
 * The footnote under the Connection group, which compares the two paths'
 * latency from this browser: "Direct 2 ms · Server 480 ms". A direct path
 * that loses probes adds its loss, "Direct 620 ms, 6% lost"; without a peer
 * only the server's path is given. Null before either path has figures.
 */
export function pathsFootnote(status: DirectStatus): SentenceText | null {
  const { direct, relay } = status.figures
  const parts: string[] = []
  if (status.peer && direct) {
    const lost = direct.loss ? `, ${formatLoss(direct.loss)} lost` : ''
    parts.push(`Direct ${formatLatency(direct.latencyMs)}${lost}`)
  }
  if (relay) {
    parts.push(`Server ${formatLatency(relay.latencyMs)}`)
  }
  return parts.length ? parts.join(' · ') : null
}

/**
 * The sentence the header gives for why the server's path is used, word for
 * word as the design's table gives it; null when the header says nothing
 * more: on Server Only, which the route below says, and before an attempt
 * has ended.
 */
export function reasonSentence(reason: DirectReason): SentenceText | null {
  switch (reason.kind) {
    case 'slower':
      return 'Direct is slower right now.'
    case 'blocked':
      return 'This browser blocks local network access.'
    case 'unreachable':
      return 'Your networks block a direct connection.'
    case 'network':
      return reason.side === 'browser' ? 'Your network blocks it.' : 'The device’s network blocks it.'
    case 'busy':
      return 'The device has too many connections.'
    case 'dropped':
      return 'The direct connection dropped.'
    case 'notOffered':
      return 'Direct works only on the device’s network.'
    case 'serverOnly':
    case 'notYet':
      return null
  }
}

/**
 * An address as the details show it: the random `….local` name a browser
 * gives its own addresses is no address the user can use, so it says so.
 */
export function shownAddress(address: string): string {
  return address.endsWith('.local') ? 'Hidden by the browser' : address
}
