import type { SentenceText } from '../ui/ui-text'

/**
 * How this page reaches a paired device, and why not directly
 * (`direct-channel.md` § What the user sees): what the page's attempts saw,
 * which the device's page shows, and the reason it gives from it.
 */

/** Where an attempt that failed stopped. */
export type DirectStage = 'permission' | 'gathering' | 'checking' | 'handshake' | 'channel'

export const DIRECT_STAGE_LABEL: Record<DirectStage, SentenceText> = {
  permission: 'Permission',
  gathering: 'Gathering addresses',
  checking: 'Finding a path',
  handshake: 'Encryption handshake',
  channel: 'Opening the channel',
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
  /** Where a failed attempt stopped. */
  stage: DirectStage | null
  browser: DirectAddresses
  device: DirectAddresses
  /** The address pairs checked, and how many of them answered. */
  pairs: { tried: number; answered: number }
  /** The browser's local network permission as the attempt ran. */
  permission: DirectPermission | null
  /** When a dropped channel ended, as an ISO 8601 timestamp. */
  endedAt?: string
}

/** What the device's page knows of the path to it. */
export interface DirectStatus {
  /** The device's `direct` switch. */
  enabled: boolean
  /** Whether this server names STUN servers, so a channel may cross networks. */
  crossing: boolean
  permission: DirectPermission | null
  /** Whether this page's channel to it stands now. */
  connected: boolean
  /** An attempt runs now. */
  trying: boolean
  /** The round trip the direct channel measured, while it stands. */
  roundTripMs: number | null
  /** The last attempt; null before the first. */
  attempt: DirectAttempt | null
  /** When the next attempt runs, as an ISO 8601 timestamp; null when none is planned. */
  nextAt: string | null
}

/** Why the page reaches a device through the server. */
export type DirectReason =
  | { kind: 'off' }
  | { kind: 'blocked' }
  | { kind: 'unreachable'; pairs: number }
  | { kind: 'network'; side: 'browser' | 'device' }
  | { kind: 'busy' }
  | { kind: 'dropped'; endedAt: string | null }
  | { kind: 'notOffered' }
  | { kind: 'notYet' }

/**
 * The reason the page reaches a device through the server, from what its
 * last attempt saw, in the order the design gives them; null while it
 * reaches it directly.
 */
export function directReason(status: DirectStatus): DirectReason | null {
  if (!status.enabled) {
    return { kind: 'off' }
  }
  if (status.connected) {
    return null
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
    case 'off':
      return 'direct connections off'
    case 'blocked':
      return 'blocked by this browser'
    case 'unreachable':
      return 'the networks don’t allow it'
    case 'network':
      return reason.side === 'browser' ? 'this network blocks it' : 'the device’s network blocks it'
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

/**
 * The sentence that says why the page goes through the server and what the
 * user can do, as the design's table words it; `time` formats a moment.
 */
export function reasonSentence(reason: DirectReason, time: (iso: string) => string): SentenceText {
  switch (reason.kind) {
    case 'off':
      return 'Direct connections are off for this device, so everything goes through the server.'
    case 'blocked':
      return 'This browser blocks local network access for this site. Allow Local network access in the site’s settings in the browser; Demi then connects without a reload.'
    case 'unreachable':
      return `Your network and the device’s don’t let a direct connection through, as a strict NAT or a firewall does: none of the ${reason.pairs} ${reason.pairs === 1 ? 'path' : 'paths'} tried answered.`
    case 'network':
      return reason.side === 'browser'
        ? 'This browser’s network blocks direct connections: it couldn’t learn its public address, as when the network blocks UDP.'
        : 'The device’s network blocks direct connections: the device couldn’t learn its public address, as when the network blocks UDP.'
    case 'busy':
      return 'The device’s runner has too many connections open. Demi tries again.'
    case 'dropped':
      return reason.endedAt
        ? `The direct connection worked until ${time(reason.endedAt)}. Demi tries again.`
        : 'The direct connection dropped. Demi tries again.'
    case 'notOffered':
      return 'On this server, direct connections work only on the same network as the device.'
    case 'notYet':
      return 'Demi hasn’t made a direct connection to this device yet.'
  }
}
