/**
 * A conversation's operations over either path (`direct-channel.md`
 * § Operations on the channel, § Choosing the path): each starts on the
 * path the device's choice names at that moment and finishes there. One
 * that fails on the direct channel sets the choice to `relay` and runs
 * again on the relay, a write once; a refusal the runner answers is the
 * operation's answer, as the relay's route would give it, unless it sends
 * the operation to the relay (`busy`, `too_large`).
 */
import type { FileReads } from '@demicodes/web-ui/files/kept-source'
import type { FileUploadOptions } from '@demicodes/web-ui/files/types'
import type { OpenUserStream, UserStream } from '@demicodes/web-ui/plugins/streams'
import { reconnectNow } from '@demicodes/web-ui/transport/liveness'
import { relayWatchLink, type OpenWatchLink, type WatchLink } from '../conversation/file-watch'
import { ApiError } from '../api/client'
import { fileBrowserError } from '../api/files'
import {
  errorCodeSchema,
  listedSchema,
  openedSchema,
  readOpenedSchema,
  textOpenedSchema,
  type ChannelHeader,
  type Opened,
  type WriteEnd,
} from '../api/generated/web-api'
import type { DirectRoute } from '.'
import { ChannelFailed, ChannelRefused, relayAnswers, type DirectPeer, type OperationChannel } from './peer'

/** The relay's answer to a refusal of the runner's: the same code, status and words. */
export function relayError(refused: ChannelRefused): ApiError {
  const code = errorCodeSchema.safeParse(refused.code)
  return new ApiError(refused.status, code.success ? code.data : null, refused.message)
}

/** The direct route of an operation as it starts; none while the conversation has none. */
export type RouteOf = () => DirectRoute | null

/**
 * Runs `direct` on the conversation's peer when the choice is `direct` as
 * the operation starts, and `relay` otherwise, or after the direct channel
 * failed or sent the operation to the relay.
 */
export async function eitherPath<T>(
  routeOf: RouteOf,
  direct: (peer: DirectPeer, route: DirectRoute) => Promise<T>,
  relay: () => Promise<T>,
): Promise<T> {
  const route = routeOf()
  const peer = route?.device.current()
  if (!route || !peer)
    return relay()
  try {
    return await direct(peer, route)
  } catch (error) {
    if (error instanceof ChannelFailed)
      route.device.failed()
    if (relayAnswers(error))
      return relay()
    if (error instanceof ChannelRefused)
      throw relayError(error)
    throw error
  }
}

/** Every byte the runner sends until it closes the channel. */
async function bytesToEnd(next: () => Promise<{ kind: 'text' | 'bytes'; bytes?: Uint8Array } | null>): Promise<Uint8Array> {
  const parts: Uint8Array[] = []
  let length = 0
  for (;;) {
    const message = await next()
    if (message === null)
      break
    if (message.kind !== 'bytes' || !message.bytes)
      throw new ChannelFailed('The runner sent text among bytes')
    parts.push(message.bytes)
    length += message.bytes.length
  }
  const bytes = new Uint8Array(length)
  let offset = 0
  for (const part of parts) {
    bytes.set(part, offset)
    offset += part.length
  }
  return bytes
}

/**
 * A conversation's file reads over either path: `relay` are the relay's
 * reads, which the direct channel's stand in for while the choice is
 * `direct`. A preview's bytes keep the relay's URL, which the service
 * worker answers directly.
 */
export function directFileReads(routeOf: RouteOf, relay: FileReads): FileReads {
  const failure = (error: unknown) => fileBrowserError(error)
  const either = async <T>(direct: (peer: DirectPeer, route: DirectRoute) => Promise<T>, onRelay: () => Promise<T>) => {
    try {
      return await eitherPath(routeOf, direct, onRelay)
    } catch (error) {
      throw failure(error)
    }
  }
  return {
    ...relay,
    list: (path) =>
      either(async (peer, route) => {
        const channel = await peer.open({ ...route.scope, op: 'list', path }, listedSchema)
        const listed = await channel.answer()
        return listed.entries.map((entry) => ({
          name: entry.name,
          isDirectory: entry.isDirectory,
          size: entry.size,
          modifiedAt: entry.modifiedAt,
        }))
      }, () => relay.list(path)),
    ...(relay.contents
      ? {
          contents: {
            ...relay.contents,
            // What a preview shows of the file: its size, time and version,
            // from a read of none of its bytes.
            describe: (path: string) =>
              either(async (peer, route) => {
                const channel = await peer.open({ ...route.scope, op: 'read', path, length: 0 }, readOpenedSchema)
                const { size, modifiedAt, version } = await channel.answer()
                channel.close()
                return { size, modifiedAt, version }
              }, () => relay.contents!.describe(path)),
          },
        }
      : {}),
    ...(relay.readText
      ? {
          readText: (path: string, held: string | null) =>
            either(async (peer, route) => {
              const text: ChannelHeader = held === null
                ? { ...route.scope, op: 'text', path }
                : { ...route.scope, op: 'text', path, version: held }
              const channel = await peer.open(text, textOpenedSchema)
              const opened = await channel.answer()
              if (opened.unchanged)
                return null
              const bytes = await bytesToEnd(() => channel.next())
              return { text: new TextDecoder().decode(bytes), version: opened.version }
            }, () => relay.readText!(path, held)),
        }
      : {}),
    ...(relay.createDirectory
      ? {
          createDirectory: (path: string, signal?: AbortSignal) =>
            either(async (peer, route) => {
              const channel = await peer.open({ ...route.scope, op: 'mkdir', path }, openedSchema)
              await channel.answer()
            }, () => relay.createDirectory!(path, signal)),
        }
      : {}),
    ...(relay.remove
      ? {
          remove: (path: string, signal?: AbortSignal) =>
            either(async (peer, route) => {
              const channel = await peer.open({ ...route.scope, op: 'delete', path }, openedSchema)
              await channel.answer()
            }, () => relay.remove!(path, signal)),
        }
      : {}),
    ...(relay.upload
      ? {
          upload: (path: string, file: File, options: FileUploadOptions) =>
            either(
              (peer, route) => writeFile(peer, route, path, file, options),
              () => relay.upload!(path, file, options),
            ),
        }
      : {}),
  }
}

/**
 * Writes `file` to `path` on a `write` channel: its bytes, as the
 * channel's queue takes them, then `{ end: true }`; the runner answers once
 * the file is in place. An abort closes the channel, which leaves the file
 * as it was.
 */
async function writeFile(
  peer: DirectPeer,
  route: DirectRoute,
  path: string,
  file: File,
  options: FileUploadOptions,
): Promise<void> {
  options.signal.throwIfAborted()
  const channel = await peer.open({ ...route.scope, op: 'write', path, replace: options.replace }, openedSchema)
  const abort = () => channel.close()
  options.signal.addEventListener('abort', abort, { once: true })
  try {
    const reader = file.stream().getReader()
    let sent = 0
    for (;;) {
      const { done, value } = await reader.read()
      if (done)
        break
      await channel.sendBytes(value)
      sent += value.length
      options.progress(sent)
    }
    const end: WriteEnd = { end: true }
    await channel.sendText(JSON.stringify(end))
    await channel.answer()
  } catch (error) {
    // An abort is the caller's, not a failure of the channel.
    options.signal.throwIfAborted()
    throw error
  } finally {
    options.signal.removeEventListener('abort', abort)
  }
}

/**
 * A plugin's user stream over either path: it opens on the path the
 * device's choice names, and when the choice changes it ends, as after a
 * lost connection, and the views waiting to reconnect connect at once, on
 * the other path (`direct-channel.md` § Choosing the path).
 */
export function directStream(routeOf: RouteOf, name: string, relay: OpenUserStream): OpenUserStream {
  return (handlers) => {
    const route = routeOf()
    const peer = route?.device.current()
    let ended = false
    let stream: UserStream
    const end = (reason: string) => {
      if (ended)
        return
      ended = true
      stopMoving()
      stream.close()
      handlers.closed(reason)
    }
    const onRelay = () =>
      relay({
        data: (bytes) => {
          if (!ended)
            handlers.data(bytes)
        },
        closed: (reason) => end(reason),
      })
    stream = route && peer ? openDirect(peer, route, name, handlers, end, () => {
      // The runner refused the stream, such as for a service it could not
      // start: the relay serves it.
      if (!ended)
        stream = onRelay()
    }) : onRelay()
    // The choice changed: this view ends, and reopens at once on the other path.
    const stopMoving = route?.device.onChange(() => {
      end('moved')
      reconnectNow()
    }) ?? (() => {})
    return {
      send: (bytes) => stream.send(bytes),
      close() {
        ended = true
        stopMoving()
        stream.close()
      },
    }
  }
}

/** A user stream on a `stream` channel; bytes sent before it opens wait for it. */
function openDirect(
  peer: DirectPeer,
  route: DirectRoute,
  name: string,
  handlers: { data(bytes: Uint8Array): void },
  end: (reason: string) => void,
  refused: () => void,
): UserStream {
  let closed = false
  const queued: Uint8Array[] = []
  let send: ((bytes: Uint8Array) => void) | null = null
  let close = () => {}
  void (async () => {
    let channel: OperationChannel<Opened> | undefined
    try {
      channel = await peer.open({ ...route.scope, op: 'stream', stream: name }, openedSchema)
      close = () => channel?.close()
      if (closed) {
        channel.close()
        return
      }
      await channel.answer()
    } catch (error) {
      if (error instanceof ChannelRefused && !closed) {
        refused()
        return
      }
      if (error instanceof ChannelFailed)
        route.device.failed()
      end('direct_failed')
      return
    }
    const open = channel
    send = (bytes) => void open.sendBytes(bytes).catch(() => {})
    for (const bytes of queued.splice(0))
      send(bytes)
    try {
      for (;;) {
        const message = await open.next()
        if (message === null) {
          end('completed')
          return
        }
        if (message.kind === 'bytes')
          handlers.data(message.bytes)
      }
    } catch {
      route.device.failed()
      end('direct_failed')
    }
  })()
  return {
    send(bytes) {
      if (send)
        send(bytes)
      else
        queued.push(bytes)
    },
    close() {
      closed = true
      close()
    },
  }
}

/**
 * A conversation's file watch over either path: a `watch` channel while the
 * choice is `direct` as it opens, which carries the relay's messages, and
 * the relay's socket otherwise or when the runner refuses it. The watch
 * moves when the choice changes (`ConversationWatch.move`).
 */
export const directWatchLink = (routeOf: RouteOf): OpenWatchLink => (conversationId, handlers) => {
  const route = routeOf()
  const peer = route?.device.current()
  if (!route || !peer)
    return relayWatchLink(conversationId, handlers)
  let closed = false
  let link: WatchLink | null = null
  const queued: string[] = []
  void (async () => {
    try {
      const channel = await peer.open({ ...route.scope, op: 'watch' }, openedSchema)
      link = {
        send: (text) => void channel.sendText(text).catch(() => {}),
        close: () => channel.close(),
      }
      if (closed) {
        link.close()
        return
      }
      await channel.answer()
      handlers.opened()
      for (const text of queued.splice(0))
        link.send(text)
      for (;;) {
        const message = await channel.next()
        if (message === null)
          break
        if (message.kind === 'text')
          handlers.message(message.text)
      }
    } catch (error) {
      if (error instanceof ChannelRefused && !closed) {
        // The runner refused the watch: the relay serves it.
        link = relayWatchLink(conversationId, handlers)
        return
      }
      if (error instanceof ChannelFailed)
        route.device.failed()
    }
    if (!closed)
      handlers.closed()
  })()
  return {
    send(text) {
      if (link)
        link.send(text)
      else
        queued.push(text)
    },
    close() {
      closed = true
      link?.close()
    },
  }
}
