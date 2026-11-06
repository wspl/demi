/**
 * The file watches of the page:
 *
 * - A conversation's, its `file_watch` lane or its direct channel's `watch`,
 *   which also carries an attached device's subscriptions under their `device`.
 * - A device's folder dialog's, its `device_watch` lane.
 *
 * Each carries the subscriptions of what the page keeps of one device's files
 * (`WatchChannel`). This module opens their connections and reads what they
 * bring against its schema.
 */
import { schemaRefusal } from '@demicodes/protocol'
import type { DeviceFiles } from '@demicodes/web-ui/files/file-cache'
import { WatchChannel, type WatchLink, type WatchLinkHandlers, type WatchRequest } from '@demicodes/web-ui/files/watch-channel'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { t } from './locales/text'
import { openLane, type LaneTarget } from '@demicodes/web-ui/transport/page-connection'
import {
  fileWatchMessageSchema,
  type ConversationWatchRequest,
  type FileWatchRequest,
} from '../api/generated/web-api'

/** Opens a connection of a conversation's watch. */
export type OpenConversationWatchLink = (conversationId: string, handlers: WatchLinkHandlers) => WatchLink

/** Opens a connection of a conversation's watch of its attached device `deviceId`. */
export type OpenAttachedWatchLink = (conversationId: string, deviceId: string, handlers: WatchLinkHandlers) => WatchLink

/**
 * Follows the device `deviceId`'s state for the page's lifetime: calls
 * `cameOnline` each time the device comes online, and answers what tells
 * whether it is online now.
 */
export type FollowDevice = (deviceId: string, cameOnline: () => void) => () => boolean

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
      reportError(t('failures.readFileWatch'), schemaRefusal(parsed.error), { site: 'failures.readFileWatch' })
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

/** A watch's message with each subscription on the attached device `device`. */
function onDevice(request: WatchRequest, device: string): ConversationWatchRequest {
  return request.type === 'subscribe'
    ? { type: 'subscribe', subscriptions: request.subscriptions.map((subscription) => ({ ...subscription, device })) }
    : request
}

/**
 * A watch on the relay, a lane of the page connection that carries what
 * `target` names; with `device`, each subscription is on that attached device.
 */
function laneWatchLink(target: LaneTarget, handlers: WatchLinkHandlers, device?: string): WatchLink {
  const lane = openLane(target, 'file watch')
  const receive = watchReceiver(handlers, () => lane.close(1000))
  lane.addEventListener('open', () => handlers.opened(false))
  lane.addEventListener('message', (event) => receive(event.data))
  lane.addEventListener('close', () => handlers.closed())
  return {
    send: (request) => lane.send(watchText(device === undefined ? request : onDevice(request, device))),
    close: () => lane.close(1000),
  }
}

/** The relay's watch of a conversation. */
export const relayWatchLink: OpenConversationWatchLink = (conversationId, handlers) =>
  laneWatchLink({ kind: 'file_watch', conversation: conversationId }, handlers)

/**
 * The relay's watch of a conversation's attached device: the conversation's
 * watch, whose subscriptions name the device. A direct channel reaches only
 * the primary device, so an attached device's watch is always on the relay.
 */
export const relayAttachedWatchLink: OpenAttachedWatchLink = (conversationId, deviceId, handlers) =>
  laneWatchLink({ kind: 'file_watch', conversation: conversationId }, handlers, deviceId)

/**
 * A conversation's file watch: one watch of the device the conversation runs
 * on, which carries the subscriptions of what the conversation's views keep
 * of it, on the relay or the direct channel as the path to the device is
 * chosen, and one of each attached device its attachment picker lists. When
 * the conversation moves to another device, the watch of the one it left lets
 * go of what it carried.
 *
 * Each Device's watch has a connection of its own: the backend's `offline`
 * names no device, and ends the connection with every subscription on it.
 *
 * Each watch opens only while its device is online, as the device's state from
 * the synchronization channel holds it, and at once when the device comes
 * online (`FollowDevice`).
 */
export class ConversationWatch {
  private current: { deviceId: string; channel: WatchChannel } | null = null
  private readonly channels = new Map<string, WatchChannel>()
  private readonly attachedChannels = new Map<string, WatchChannel>()
  /** Whether each device a watch serves is online, from its follow. */
  private readonly devices = new Map<string, () => boolean>()

  constructor(
    private readonly conversationId: string,
    private readonly openLink: OpenConversationWatchLink = relayWatchLink,
    private readonly openAttachedLink: OpenAttachedWatchLink = relayAttachedWatchLink,
    private readonly followDevice: FollowDevice = () => () => true,
  ) {}

  /** Whether the device `deviceId` is online, following it from the first time it is asked. */
  private deviceOnline(deviceId: string): boolean {
    let online = this.devices.get(deviceId)
    if (!online) {
      online = this.followDevice(deviceId, () => this.online(deviceId))
      this.devices.set(deviceId, online)
    }
    return online()
  }

  /** The watch of the attached device `deviceId`, whose kept files are `files`. */
  attached(deviceId: string, files: DeviceFiles): WatchChannel {
    let channel = this.attachedChannels.get(deviceId)
    if (!channel) {
      channel = new WatchChannel(
        files,
        (handlers) => this.openAttachedLink(this.conversationId, deviceId, handlers),
        undefined,
        () => this.deviceOnline(deviceId),
      )
      this.attachedChannels.set(deviceId, channel)
    }
    return channel
  }

  /** The watch of the conversation's device, the device `deviceId` whose kept files are `files`. */
  channel(deviceId: string, files: DeviceFiles): WatchChannel {
    let channel = this.channels.get(deviceId)
    if (!channel) {
      channel = new WatchChannel(
        files,
        (handlers) => this.openLink(this.conversationId, handlers),
        undefined,
        () => this.deviceOnline(deviceId),
      )
      this.channels.set(deviceId, channel)
    }
    if (this.current?.deviceId !== deviceId) {
      this.current?.channel.retire()
      this.current = { deviceId, channel }
    }
    return channel
  }

  /**
   * The device `deviceId` came online, as a stopped Cloud that started: the
   * watch of it as the conversation's device or as an attached device, waiting to
   * connect again or dormant, connects now.
   */
  online(deviceId: string): void {
    this.channels.get(deviceId)?.online()
    this.attachedChannels.get(deviceId)?.online()
  }

  /**
   * The path to the device changed (`packages/web/src/direct/index.ts`): the watch reopens on the other path at once and subscribes again
   * to what it carries.
   */
  move(): void {
    this.current?.channel.move()
  }
}

/** Each device's folder dialog watch, for the page's lifetime. */
const deviceWatches = new Map<string, WatchChannel>()

/**
 * A device's folder dialog's watch, its `device_watch` lane, through device
 * access, which carries the subscriptions of what the
 * dialog keeps of the device's files.
 *
 * It opens only while the device is online, as `followDevice` holds it, and at
 * once when the device comes online; the first call's `followDevice` follows
 * the device for the page's lifetime.
 */
export function deviceWatch(deviceId: string, files: DeviceFiles, followDevice: FollowDevice): WatchChannel {
  let channel = deviceWatches.get(deviceId)
  if (!channel) {
    let online: (() => boolean) | null = null
    const opened = new WatchChannel(
      files,
      (handlers) => laneWatchLink({ kind: 'device_watch', device: deviceId }, handlers),
      undefined,
      () => (online ??= followDevice(deviceId, () => opened.online()))(),
    )
    channel = opened
    deviceWatches.set(deviceId, channel)
  }
  return channel
}
