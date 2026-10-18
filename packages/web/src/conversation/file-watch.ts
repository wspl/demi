/**
 * The file watches of the page (`web-api-hosts.md` § File watch): a
 * conversation's, `WS /conversations/:id/fs/watch` or its direct channel's
 * `watch`, which also carries an attached host's subscriptions under their
 * `host`, and a device's folder dialog's, `WS /devices/:deviceId/fs/watch`.
 * Each carries the subscriptions of what the page keeps of one Host's files
 * (`WatchChannel`); this module opens their connections and reads what they
 * bring against its schema.
 */
import { schemaRefusal } from '@demicodes/protocol'
import type { HostFiles } from '@demicodes/web-ui/files/file-cache'
import { WatchChannel, type WatchLink, type WatchLinkHandlers, type WatchRequest } from '@demicodes/web-ui/files/watch-channel'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { t } from './locales/text'
import { openSocket, pingOn } from '@demicodes/web-ui/transport/liveness'
import { apiUrl } from '../api/client'
import {
  fileWatchMessageSchema,
  type ConversationWatchRequest,
  type FileWatchRequest,
} from '../api/generated/web-api'

/** Opens a connection of a conversation's watch. */
export type OpenConversationWatchLink = (conversationId: string, handlers: WatchLinkHandlers) => WatchLink

/** Opens a connection of a conversation's watch of its attached host `deviceId`. */
export type OpenAttachedWatchLink = (conversationId: string, deviceId: string, handlers: WatchLinkHandlers) => WatchLink

/**
 * Reads each message of a watch against its schema and hands it on; one
 * that does not read is for a developer, and ends the connection, which
 * opens again after its wait.
 */
export function watchReceiver(handlers: WatchLinkHandlers, close: () => void): (data: unknown) => void {
  let ended = false
  return (data) => {
    if (ended)
      return
    let value: unknown
    try {
      value = typeof data === 'string' ? JSON.parse(data) : null
    } catch {
      value = null
    }
    const parsed = fileWatchMessageSchema.safeParse(value)
    if (!parsed.success) {
      reportError(t('failures.readFileWatch'), schemaRefusal(parsed.error))
      ended = true
      close()
      handlers.closed()
      return
    }
    handlers.message(parsed.data)
  }
}

/** A message of the page's as its watch carries it, checked against the protocol's type. */
export function watchText(request: FileWatchRequest | ConversationWatchRequest): string {
  return JSON.stringify(request)
}

/** A watch's message with each subscription on the attached host `host`. */
function onHost(request: WatchRequest, host: string): ConversationWatchRequest {
  return request.type === 'subscribe'
    ? { type: 'subscribe', subscriptions: request.subscriptions.map((subscription) => ({ ...subscription, host })) }
    : request
}

/**
 * A watch on the relay's WebSocket at the API path `path`; with `host`,
 * each subscription is on that attached host.
 */
function socketWatchLink(path: string, handlers: WatchLinkHandlers, host?: string): WatchLink {
  const url = new URL(apiUrl(path), window.location.href)
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  const socket = openSocket(url, 'file watch')
  const receive = watchReceiver(handlers, () => socket.close(1000))
  socket.addEventListener('open', () => handlers.opened(pingOn(socket)))
  socket.addEventListener('message', (event) => receive(event.data))
  socket.addEventListener('close', () => handlers.closed())
  return {
    send: (request) => socket.send(watchText(host === undefined ? request : onHost(request, host))),
    close: () => socket.close(1000),
  }
}

/** The relay's watch of a conversation, `WS /conversations/:id/fs/watch`. */
export const relayWatchLink: OpenConversationWatchLink = (conversationId, handlers) =>
  socketWatchLink(`/conversations/${encodeURIComponent(conversationId)}/fs/watch`, handlers)

/**
 * The relay's watch of a conversation's attached host: the conversation's
 * socket, whose subscriptions name the host. A direct channel reaches only
 * the primary Host, so an attached host's watch is always on the relay.
 */
export const relayAttachedWatchLink: OpenAttachedWatchLink = (conversationId, deviceId, handlers) =>
  socketWatchLink(`/conversations/${encodeURIComponent(conversationId)}/fs/watch`, handlers, deviceId)

/**
 * A conversation's file watch: one watch of the Host the conversation runs
 * on, which carries the subscriptions of what the conversation's views keep
 * of it, on the relay or the direct channel as the path to the device is
 * chosen, and one of each attached host its attachment picker lists. When
 * the conversation moves to another Host, the watch of the one it left lets
 * go of what it carried.
 *
 * Each Host's watch has a connection of its own: the backend's `offline`
 * names no Host, and ends the connection with every subscription on it.
 */
export class ConversationWatch {
  private current: { deviceId: string; channel: WatchChannel } | null = null
  private readonly channels = new Map<string, WatchChannel>()
  private readonly attachedChannels = new Map<string, WatchChannel>()

  constructor(
    private readonly conversationId: string,
    private readonly openLink: OpenConversationWatchLink = relayWatchLink,
    private readonly openAttachedLink: OpenAttachedWatchLink = relayAttachedWatchLink,
  ) {}

  /** The watch of the attached host `deviceId`, whose kept files are `files`. */
  attached(deviceId: string, files: HostFiles): WatchChannel {
    let channel = this.attachedChannels.get(deviceId)
    if (!channel) {
      channel = new WatchChannel(files, (handlers) => this.openAttachedLink(this.conversationId, deviceId, handlers))
      this.attachedChannels.set(deviceId, channel)
    }
    return channel
  }

  /** The attached host `deviceId` came online, as a stopped Cloud that started: its watch connects at once. */
  attachedOnline(deviceId: string): void {
    this.attachedChannels.get(deviceId)?.online()
  }

  /** The watch of the conversation's Host, the device `deviceId` whose kept files are `files`. */
  channel(deviceId: string, files: HostFiles): WatchChannel {
    let channel = this.channels.get(deviceId)
    if (!channel) {
      channel = new WatchChannel(files, (handlers) => this.openLink(this.conversationId, handlers))
      this.channels.set(deviceId, channel)
    }
    if (this.current?.deviceId !== deviceId) {
      this.current?.channel.retire()
      this.current = { deviceId, channel }
    }
    return channel
  }

  /** The Host came online: a watch waiting to connect again connects now. */
  online(): void {
    this.current?.channel.online()
  }

  /**
   * The path to the Host changed (`direct-channel.md` § Choosing the
   * path): the watch reopens on the other path at once and subscribes again
   * to what it carries.
   */
  move(): void {
    this.current?.channel.move()
  }
}

/** Each device's folder dialog watch, for the page's lifetime. */
const deviceWatches = new Map<string, WatchChannel>()

/**
 * A device's folder dialog's watch, `WS /devices/:deviceId/fs/watch`,
 * through device access, which carries the subscriptions of what the
 * dialog keeps of the device's files.
 */
export function deviceWatch(deviceId: string, files: HostFiles): WatchChannel {
  let channel = deviceWatches.get(deviceId)
  if (!channel) {
    channel = new WatchChannel(files, (handlers) =>
      socketWatchLink(`/devices/${encodeURIComponent(deviceId)}/fs/watch`, handlers))
    deviceWatches.set(deviceId, channel)
  }
  return channel
}
