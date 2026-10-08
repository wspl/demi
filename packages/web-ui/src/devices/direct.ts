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
  automatic: 'Demi connects directly when that is faster, and through the server otherwise.',
  direct: 'Demi connects directly whenever it can, even when the server’s path is faster.',
  server: 'Everything goes through the server, and Demi makes no direct connection.',
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
  /** The pair in use while it was connected: this browser's address, null where the browser keeps it, and the device's. */
  pair: { browser: string | null; device: string } | null
  /** The browser's local network permission as the attempt ran. */
  permission: DirectPermission | null
  /** When a dropped channel ended, as an ISO 8601 timestamp. */
  endedAt?: string
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

/** Why the page reaches a device through the server. */
export type DirectReason =
  | { kind: 'serverOnly' }
  | { kind: 'slower'; direct: PathFigures | null; relay: PathFigures | null }
  | { kind: 'blocked' }
  | { kind: 'unreachable'; pairs: number }
  | { kind: 'network'; side: 'browser' | 'device' }
  | { kind: 'busy' }
  | { kind: 'dropped'; endedAt: string | null }
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
    return { kind: 'slower', direct: status.figures.direct, relay: status.figures.relay }
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
    return { kind: 'dropped', endedAt: attempt.endedAt ?? null }
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
  return { kind: 'unreachable', pairs: attempt.pairs.tried }
}

/** The few words a device's row gives after Through the server; empty when there is nothing to say. */
export function reasonShort(reason: DirectReason): SentenceText {
  switch (reason.kind) {
    case 'serverOnly':
      return 'set to Server Only'
    case 'slower':
      return 'direct is slower right now'
    case 'blocked':
      return 'blocked by this browser'
    case 'unreachable':
      return 'the networks don’t allow direct'
    case 'network':
      return reason.side === 'browser' ? 'this network blocks direct' : 'the device’s network blocks direct'
    case 'busy':
      return 'the device is busy'
    case 'dropped':
      return 'the direct connection dropped'
    case 'notOffered':
      return 'not on the device’s network'
    case 'notYet':
      return ''
  }
}

/** A figure in whole units, with one decimal only under one: 0.4, 2, 48. */
function wholeUnlessSmall(value: number): string {
  const tenths = Math.round(value * 10) / 10
  return tenths > 0 && tenths < 1 ? String(tenths) : String(Math.round(value))
}

/** A latency as a person reads it: "0.4 ms", "2 ms", "48 ms". */
export function formatLatency(ms: number): string {
  return ms < 0.05 ? 'under 0.1 ms' : `${wholeUnlessSmall(ms)} ms`
}

/** A share lost as a person reads it: "0%", "0.5%", "6%". */
export function formatLoss(loss: number): string {
  return `${wholeUnlessSmall(loss * 100)}%`
}

/**
 * The footnote under the Connection group, which compares the two paths'
 * latency from this browser: "From this browser: directly 2 ms, through the
 * server 480 ms." A direct path that loses probes says how many, and one
 * with no peer says there is no direct connection. Null before either path
 * has figures.
 */
export function pathsFootnote(status: DirectStatus): SentenceText | null {
  const { direct, relay } = status.figures
  const parts: string[] = []
  if (status.peer && direct) {
    const lost = direct.loss ? ` with ${formatLoss(direct.loss)} lost` : ''
    parts.push(`directly ${formatLatency(direct.latencyMs)}${lost}`)
  }
  if (relay) {
    parts.push(`through the server ${formatLatency(relay.latencyMs)}`)
  }
  if (parts.length === 0) {
    return null
  }
  const none = status.peer ? '' : '; no direct connection'
  return `From this browser: ${parts.join(', ')}${none}.`
}

/** What makes the direct path worse, for the sentence that says so. */
function slowerBecause(direct: PathFigures | null, relay: PathFigures | null): string {
  if (direct?.loss && direct.loss > 0.02) {
    return `it is losing ${formatLoss(direct.loss)} of its packets`
  }
  if (direct && relay) {
    return `its round trip is ${formatLatency(direct.latencyMs)}, against ${formatLatency(relay.latencyMs)} through the server`
  }
  return 'it is slower than the server’s path'
}

/**
 * The sentence the header gives for why the server's path is used and what
 * the user can do, as the design's table words it; `time` formats a moment.
 * Null when there is nothing to explain.
 */
export function reasonSentence(reason: DirectReason, time: (iso: string) => string): SentenceText | null {
  switch (reason.kind) {
    case 'serverOnly':
      return 'The route is Server Only, so everything goes through the server.'
    case 'slower':
      return `The direct connection is up but slower right now: ${slowerBecause(reason.direct, reason.relay)}. Demi moves back when it improves.`
    case 'blocked':
      return 'This browser blocks local network access for this site. Allow Local network access in the site’s settings in the browser; Demi then connects without a reload.'
    case 'unreachable':
      return 'Your network and the device’s don’t let a direct connection through, as a strict NAT or a firewall does.'
    case 'network':
      return reason.side === 'browser'
        ? 'This browser’s network blocks direct connections: it couldn’t learn its public address, as when the network blocks UDP.'
        : 'The device’s network blocks direct connections: the device couldn’t learn its public address, as when the network blocks UDP.'
    case 'busy':
      return 'The device’s runner has too many connections open.'
    case 'dropped':
      return reason.endedAt ? `The direct connection worked until ${time(reason.endedAt)}.` : 'The direct connection dropped.'
    case 'notOffered':
      return 'On this server, direct connections work only on the same network as the device.'
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
