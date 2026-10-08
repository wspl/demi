import type { SentenceText, TitleText } from '../ui/ui-text'

/**
 * How this page reaches a paired device, and why not directly
 * (`direct-channel.md` § What the user sees): the device's route, what the
 * page's attempts saw and what it measures of both paths, which the
 * device's page shows, and the reason it gives from them.
 */

/**
 * How pages reach a device: Automatic, Prefer P2P or Relay Only. The page
 * calls the direct channel P2P and the backend's path the relay; the values
 * are the wire's and stay as published.
 */
export type DeviceRoute = 'automatic' | 'direct' | 'server'

export const DEVICE_ROUTE_LABEL: Record<DeviceRoute, TitleText> = {
  automatic: 'Automatic',
  direct: 'Prefer P2P',
  server: 'Relay Only',
}

/** The line under the route that says what the chosen one does. */
export const DEVICE_ROUTE_DESCRIPTION: Record<DeviceRoute, SentenceText> = {
  automatic: 'Uses the faster path.',
  direct: 'P2P whenever it connects.',
  server: 'Never uses P2P.',
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
  /** The device's address and port in use while it was connected. */
  inUse: { address: string; port: number } | null
  /** The browser's local network permission as the attempt ran. */
  permission: DirectPermission | null
}

/**
 * One path's figures from a measurement (`direct-channel.md` § Measuring
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
  /** Whether the page uses the peer now: connected via P2P. */
  chosen: boolean
  /**
   * An attempt runs now, whoever started it. The host sets it as it starts
   * one, before the page next renders, so Try Again knows whether its click
   * started an attempt (`useAsked`).
   */
  trying: boolean
  /** The last attempt; null before the first. */
  attempt: DirectAttempt | null
  /** When the next attempt runs, as an ISO 8601 timestamp; null when none is planned. */
  nextAt: string | null
  /** Each path's figures from the last measurement, null before one found any. */
  figures: { direct: PathFigures | null; relay: PathFigures | null }
  /** A measurement runs, whoever asked for it; the host sets it as it starts one, as `trying`. */
  measuring: boolean
}

/**
 * Why the page reaches a device via the relay, one per row of the
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
 * The reason the page reaches a device via the relay, from its route,
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
 * The Latency row's value, which compares the two paths' latency from this
 * browser's last measurement, the peer's path by its kind: "LAN 2 ms ·
 * Relay 480 ms". A peer's path that lost probes adds its loss, "P2P 620 ms,
 * 10% lost"; without a peer only the relay is given. Null before either
 * path has figures.
 */
export function pathsLatency(status: DirectStatus): SentenceText | null {
  const { direct, relay } = status.figures
  const parts: string[] = []
  if (status.peer && direct) {
    const lost = direct.loss ? `, ${formatLoss(direct.loss)} lost` : ''
    parts.push(`${DEVICE_PATH_LABEL[peerKind(status) ?? 'internet']} ${formatLatency(direct.latencyMs)}${lost}`)
  }
  if (relay) {
    parts.push(`Relay ${formatLatency(relay.latencyMs)}`)
  }
  return parts.length ? parts.join(' · ') : null
}

/**
 * The sentence the header gives for why the server's path is used, word for
 * word as the design's table gives it; null when the header says nothing
 * more: on Relay Only, which the route below says, and before an attempt
 * has ended.
 */
export function reasonSentence(reason: DirectReason): SentenceText | null {
  switch (reason.kind) {
    case 'slower':
      return 'P2P is slower right now.'
    case 'blocked':
      return 'This browser blocks local network access.'
    case 'unreachable':
      return 'Your networks block P2P connections.'
    case 'network':
      return reason.side === 'browser' ? 'Your network blocks it.' : 'The device’s network blocks it.'
    case 'busy':
      return 'The device has too many connections.'
    case 'dropped':
      return 'The P2P connection dropped.'
    case 'notOffered':
      return 'P2P works only on the device’s network.'
    case 'serverOnly':
    case 'notYet':
      return null
  }
}

/** An address with its port, as the Details sheet gives the one in use: `127.0.0.1:60044`, `[::1]:60044`. */
export function addressWithPort(inUse: { address: string; port: number }): string {
  return inUse.address.includes(':') ? `[${inUse.address}]:${inUse.port}` : `${inUse.address}:${inUse.port}`
}

/** The kind of path a P2P connection runs on (`direct-channel.md` § What the user sees). */
export type PathKind = 'thisComputer' | 'localNetwork' | 'internet'
/** The path the page uses for a device: a P2P one of its kind, or the relay. */
export type DevicePath = PathKind | 'relay'

/** Each path as its label names it, wherever the path is named: "Connected via LAN". */
export const DEVICE_PATH_LABEL: Record<DevicePath, TitleText> = {
  thisComputer: 'This Computer',
  localNetwork: 'LAN',
  internet: 'P2P',
  relay: 'Relay',
}

/** The 16-bit groups of an IPv6 address, or null for any other text. */
function ipv6Groups(address: string): number[] | null {
  // The URL parser expands and checks an IPv6 address; a zone (`%en0`) belongs to no URL.
  const host = URL.parse(`http://[${address.replace(/^\[|\]$/g, '').split('%')[0]}]/`)?.hostname
  if (!host) {
    return null
  }
  const [head = '', tail = ''] = host.slice(1, -1).split('::')
  const part = (text: string) => (text ? text.split(':').map((group) => Number.parseInt(group, 16)) : [])
  const before = part(head)
  const after = part(tail)
  return host.includes('::') ? [...before, ...Array(8 - before.length - after.length).fill(0), ...after] : before
}

/** The four bytes of an IPv4 address, or null for any other text. */
function ipv4Bytes(address: string): number[] | null {
  const parts = address.split('.')
  if (parts.length !== 4 || !parts.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255)) {
    return null
  }
  return parts.map(Number)
}

/**
 * The kind of path the device's address in use names: a loopback address is
 * this computer; a private, link-local or shared one, `100.64/10` included,
 * which VPNs such as Tailscale use, is the local network; any other is the
 * internet.
 */
export function pathKind(address: string): PathKind {
  const ipv6 = ipv4Bytes(address) === null ? ipv6Groups(address) : null
  // An IPv4 address in IPv6 form (`::ffff:a.b.c.d`) is that IPv4 address.
  const mapped = ipv6 && ipv6.slice(0, 5).every((group) => group === 0) && ipv6[5] === 0xffff
    ? [ipv6[6]! >> 8, ipv6[6]! & 0xff, ipv6[7]! >> 8, ipv6[7]! & 0xff]
    : null
  const v4 = ipv4Bytes(address) ?? mapped
  if (v4) {
    const [a = 0, b = 0] = v4
    if (a === 127) {
      return 'thisComputer'
    }
    const local = a === 10
      || (a === 172 && b >= 16 && b <= 31)
      || (a === 192 && b === 168)
      || (a === 169 && b === 254)
      || (a === 100 && b >= 64 && b <= 127)
    return local ? 'localNetwork' : 'internet'
  }
  if (ipv6) {
    if (ipv6.slice(0, 7).every((group) => group === 0) && ipv6[7] === 1) {
      return 'thisComputer'
    }
    const first = ipv6[0]!
    return (first & 0xfe00) === 0xfc00 || (first & 0xffc0) === 0xfe80 ? 'localNetwork' : 'internet'
  }
  return 'internet'
}

/** How an online device is reached, as its row and its page's header say: "Connected via LAN", "Connected via relay". */
export function connectedVia(status: DirectStatus): SentenceText {
  const path = statusPath(status)
  return path === 'relay' ? 'Connected via relay' : `Connected via ${DEVICE_PATH_LABEL[path]}`
}

/** The P2P row's subtitle while connected: "LAN · 192.168.1.20". */
export function pathSentence(address: string): SentenceText {
  return `${DEVICE_PATH_LABEL[pathKind(address)]} · ${address}`
}

/** The kind of the connected peer's path, from its address in use; null without a peer. */
function peerKind(status: DirectStatus): PathKind | null {
  const inUse = status.attempt?.inUse
  return status.peer && inUse ? pathKind(inUse.address) : null
}

/** The path a page reaching a device with `status` uses: its peer's kind while it uses the peer, the relay otherwise. */
export function statusPath(status: DirectStatus): DevicePath {
  return (status.chosen && peerKind(status)) || 'relay'
}


/**
 * An address as the details show it: the random `….local` name a browser
 * gives its own addresses is no address the user can use, so it says so.
 */
export function shownAddress(address: string): string {
  return address.endsWith('.local') ? 'Hidden by the browser' : address
}
