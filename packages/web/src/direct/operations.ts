/**
 * A conversation's operations over either path (`direct-channel.md`
 * § Operations on the channel, § Choosing the path): each starts on the
 * path the device's choice names at that moment and finishes there. One
 * that fails on the direct channel sets the choice to `relay` and runs
 * again on the relay, a write once; a refusal the runner answers is the
 * operation's answer, as the relay's route would give it, unless it sends
 * the operation to the relay (`busy`, `too_large`).
 */
import type { Read } from '@demicodes/web-ui/files/file-cache'
import type { FileReads } from '@demicodes/web-ui/files/kept-source'
import type { FileText, FileUploadOptions } from '@demicodes/web-ui/files/types'
import type { OpenUserStream, StreamBytes, UserStream, UserStreamHandlers } from '@demicodes/web-ui/plugins/streams'
import { LiveFrameSplitter, frameKind } from '@demicodes/web-ui/live/frames'
import { REFUSED_FRAME } from '@demicodes/web-ui/live/link'
import { LIVE_ACK_FRAME } from '@demicodes/protocol'
import type { WatchLink } from '@demicodes/web-ui/files/watch-channel'
import { reconnectNow } from '@demicodes/web-ui/transport/liveness'
import { relayWatchLink, watchReceiver, watchText, type OpenConversationWatchLink } from '../conversation/file-watch'
import { ApiError } from '../api/client'
import { fileBrowserEntry, fileBrowserError } from '../api/files'
import {
  errorCodeSchema,
  listedSchema,
  openedSchema,
  readOpenedSchema,
  textOpenedSchema,
  videoOpenedSchema,
  type ChannelHeader,
  type Opened,
  type WriteEnd,
} from '../api/generated/web-api'
import type { DirectRoute } from '.'
import { ChannelFailed, ChannelRefused, relayAnswers, type DirectPeer, type OperationChannel } from './peer'

/**
 * The relay's answer to a refusal of the runner's: the same code, status
 * and words, and the ETag and `X-Demi-Read-At` the relay's answer carries
 * for a file that is not text or too large to read as text.
 */
export function relayError(refused: ChannelRefused): ApiError {
  const code = errorCodeSchema.safeParse(refused.code)
  const headers = new Headers()
  const { version, readAt } = refused.facts
  if (version)
    headers.set('etag', version)
  if (readAt)
    headers.set('x-demi-read-at', `${readAt.epoch}.${readAt.number}`)
  return new ApiError(refused.status, code.success ? code.data : null, refused.message, null, null, headers)
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
        return { value: listed.entries.map(fileBrowserEntry), readAt: listed.readAt }
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
                const { size, modifiedAt, version, readAt } = await channel.answer()
                channel.close()
                return { value: { size, modifiedAt, version }, readAt }
              }, () => relay.contents!.describe(path)),
          },
        }
      : {}),
    ...(relay.readText
      ? {
          readText: (path: string, held: string | null) =>
            either<Read<FileText>>(async (peer, route) => {
              const text: ChannelHeader = held === null
                ? { ...route.scope, op: 'text', path }
                : { ...route.scope, op: 'text', path, version: held }
              const channel = await peer.open(text, textOpenedSchema)
              const opened = await channel.answer()
              if (opened.unchanged)
                return { unchanged: true, readAt: opened.readAt }
              const bytes = await bytesToEnd(() => channel.next())
              return { value: { text: new TextDecoder().decode(bytes), version: opened.version }, readAt: opened.readAt }
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
 * the other path (`direct-channel.md` § Choosing the path). A live view's
 * stream goes on the device's live peer (`live.ts`).
 */
export function directStream(routeOf: RouteOf, name: string, relay: OpenUserStream): OpenUserStream {
  return (handlers, options) => {
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
    // The runner refused the stream, such as for a service it could not
    // start or a peer at its limit of channels: the relay serves it.
    // Bytes the view sent before go there too, its first messages among them.
    const refused = (unsent: StreamBytes[]) => {
      if (ended)
        return
      stream = onRelay()
      for (const bytes of unsent)
        stream.send(bytes)
    }
    if (!route || !peer)
      stream = onRelay()
    else if (options?.live)
      stream = openLive(route, name, handlers, end, refused)
    else
      stream = openDirect(peer, route, name, handlers, end, refused)
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

/**
 * A live view's stream on the device's live peer: its `video` channel opens first, then its `stream`
 * channel naming it. The pictures' chunks come on `video`, each message one
 * chunk, and the page's acknowledgements go there; everything else goes on
 * `stream`, whose messages the page joins into whole frames before the view
 * reads them, so that a chunk never falls inside another frame. A live peer
 * that cannot be made, or a refusal, leaves the view to the relay; bytes
 * sent before the channels open wait for them. A channel that fails sets
 * the device's choice to `relay`, where the view reopens; closing the view
 * closes its channels, opening ones included.
 */
function openLive(
  route: DirectRoute,
  name: string,
  handlers: UserStreamHandlers,
  end: (reason: string) => void,
  refused: (unsent: StreamBytes[]) => void,
): UserStream {
  const lease = route.live.use()
  let closed = false
  const queued: StreamBytes[] = []
  let send: ((bytes: StreamBytes) => void) | null = null
  const channels: OperationChannel<unknown>[] = []
  // Closes the channels still opening, which `channels` holds only once open.
  const opening = new AbortController()
  const close = () => {
    opening.abort()
    for (const channel of channels)
      channel.close()
    lease.release()
  }
  const finish = (reason: string) => {
    close()
    end(reason)
  }
  void (async () => {
    let peer: DirectPeer
    let video: OperationChannel<unknown>
    let control: OperationChannel<unknown>
    try {
      peer = await lease.peer
    } catch {
      // No live peer now: the relay serves this view, and the next one tries again.
      lease.release()
      if (!closed)
        refused(queued.splice(0))
      return
    }
    // A channel of the live peer failed: the relay is the choice, which ends
    // this view as `moved`. A channel the view's own close ended did not fail.
    const failed = () => {
      if (closed)
        return
      route.live.failed(peer)
      route.device.failed()
      if (!closed)
        finish('direct_failed')
    }
    if (closed)
      return
    try {
      const opened = await peer.open({ op: 'video' }, videoOpenedSchema, false, opening.signal)
      channels.push(opened)
      if (closed)
        return close()
      const { id } = await opened.answer()
      video = opened
      const stream = await peer.open({ ...route.scope, op: 'stream', stream: name, video: id }, openedSchema, true, opening.signal)
      channels.push(stream)
      if (closed)
        return close()
      await stream.answer()
      control = stream
    } catch (error) {
      if (error instanceof ChannelRefused && !closed) {
        close()
        refused(queued.splice(0))
        return
      }
      if (error instanceof ChannelFailed)
        failed()
      else if (!closed)
        finish('direct_failed')
      return
    }
    send = (bytes) => {
      const channel = frameKind(bytes) === LIVE_ACK_FRAME ? video : control
      void channel.sendBytes(bytes).catch(() => {})
    }
    for (const bytes of queued.splice(0))
      send(bytes)
    // The stream's end ends the view; its video channel ends with it.
    void (async () => {
      for (;;) {
        const message = await video.next()
        if (message === null || closed)
          return
        if (message.kind === 'bytes')
          handlers.data(message.bytes)
      }
    })().catch(failed)
    try {
      const frames = new LiveFrameSplitter()
      for (;;) {
        const message = await control.next()
        if (closed)
          return
        if (message === null) {
          finish('completed')
          return
        }
        if (message.kind === 'bytes') {
          let whole: Uint8Array[]
          try {
            whole = frames.split(message.bytes)
          } catch {
            // The runner's module ships with this page, so a frame of an impossible size is its defect, as the view would take it.
            finish(REFUSED_FRAME)
            return
          }
          for (const frame of whole)
            handlers.data(frame)
        }
      }
    } catch {
      failed()
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
      if (closed)
        return
      closed = true
      close()
    },
  }
}

/** A user stream on a `stream` channel; bytes sent before it opens wait for it. */
function openDirect(
  peer: DirectPeer,
  route: DirectRoute,
  name: string,
  handlers: { data(bytes: Uint8Array): void },
  end: (reason: string) => void,
  refused: (unsent: StreamBytes[]) => void,
): UserStream {
  let closed = false
  const queued: StreamBytes[] = []
  let send: ((bytes: StreamBytes) => void) | null = null
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
        refused(queued.splice(0))
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
export const directWatchLink = (routeOf: RouteOf): OpenConversationWatchLink => (conversationId, handlers) => {
  const route = routeOf()
  const peer = route?.device.current()
  if (!route || !peer)
    return relayWatchLink(conversationId, handlers)
  let closed = false
  let link: WatchLink | null = null
  const queued: Parameters<WatchLink['send']>[0][] = []
  void (async () => {
    try {
      const channel = await peer.open({ ...route.scope, op: 'watch' }, openedSchema)
      link = {
        // A message lost with a failing channel is sent again on the next connection.
        send: (request) => void channel.sendText(watchText(request)).catch(() => {}),
        close: () => channel.close(),
      }
      if (closed) {
        link.close()
        return
      }
      await channel.answer()
      handlers.opened()
      for (const request of queued.splice(0))
        link.send(request)
      const receive = watchReceiver(handlers, () => {
        closed = true
        channel.close()
      })
      for (;;) {
        const message = await channel.next()
        if (message === null || closed)
          break
        if (message.kind === 'text')
          receive(message.text)
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
    send(request) {
      if (link)
        link.send(request)
      else
        queued.push(request)
    },
    close() {
      closed = true
      link?.close()
    },
  }
}
