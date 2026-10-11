import { reactive } from 'vue'
import { z } from 'zod'
import type { Coverage, HostFiles } from '@demicodes/web-ui/files/file-cache'
import type { FileFollower } from '@demicodes/web-ui/files/kept-source'
import { parentPath } from '@demicodes/web-ui/files/paths'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { openSocket, waitToReconnect, watchSilence, type ReconnectWait, type SilenceWatch } from '@demicodes/web-ui/transport/liveness'
import { apiUrl } from '../api/client'
import { fileWatchMessageSchema, type FileWatchMessage, type FileWatchRequest } from '../api/generated/web-api'

/** The most subscriptions outside the working tree a page holds (`web-api.md` § File watch). */
const MAX_OUTSIDE = 64

/** The key of the working tree's own subscription. */
const TREE = 'tree'

/** How often the page renews what it holds; a lease runs 10 minutes (`web-api.md` § File watch). */
const RENEW_MS = 3 * 60 * 1000

/** What the page says when the Host's watch gives no reason. */
const NO_REASON = 'The Host cannot watch its files.'

/** Where a conversation's watch reaches: the Host's kept files and the working tree's root. */
export interface WatchTarget {
  files: HostFiles
  root: string
}

/** Whether `path` is `root` or lies below it. */
function within(path: string, root: string): boolean {
  const prefix = root.endsWith('/') ? root : `${root}/`
  return path === root || path.startsWith(prefix)
}

/** One connection of the watch, a WebSocket of the relay's or a direct channel's `watch`. */
export interface WatchLink {
  send(text: string): void
  close(): void
}

/** What a watch's connection tells the watch. */
export interface WatchLinkHandlers {
  /** It opened: what it says from now on counts. */
  opened(): void
  message(data: unknown): void
  /** It closed, or failed. */
  closed(): void
}

/** Opens a connection of a conversation's watch. */
export type OpenWatchLink = (conversationId: string, handlers: WatchLinkHandlers) => WatchLink

/** The relay's watch, `WS /conversations/:id/fs/watch` (`web-api.md` § File watch). */
export const relayWatchLink: OpenWatchLink = (conversationId, handlers) => {
  const url = new URL(apiUrl(`/conversations/${encodeURIComponent(conversationId)}/fs/watch`), window.location.href)
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  const socket = openSocket(url, 'file watch')
  socket.addEventListener('open', () => handlers.opened())
  socket.addEventListener('message', (event) => handlers.message(event.data))
  socket.addEventListener('close', () => handlers.closed())
  return {
    send: (text) => socket.send(text),
    close: () => socket.close(1000),
  }
}

/**
 * One connection of the watch, from its open to its close: what it covers
 * once its subscriptions are live, and what it holds.
 *
 * TODO(file-watch page package): the page subscribes here to the working
 * tree and to each shown path outside it, and a change of the tree reads
 * again what lies under it. The files service is to hold a subscription of
 * the right kind for each entry it keeps and to compare numbers
 * (`plugin-pages.md` § What the service keeps).
 */
class WatchSocket {
  readonly coverage: Coverage
  /** The paths outside the working tree whose subscriptions are live. */
  covered = new Set<string>()
  /** The paths outside the working tree it subscribed to, the latest last. */
  held: string[] = []
  /** The working tree's subscription is live. */
  live = false
  /** Watches the socket's silence from its open; a heartbeat comes after 30 s of nothing else. */
  silence: SilenceWatch | null = null
  /** Renews what it holds while it is open. */
  renewal: ReturnType<typeof setInterval> | null = null

  /** Set once the connection is made, before anything it says counts. */
  link: WatchLink | null = null
  /** The connection opened: what it holds may be sent. */
  opened = false

  constructor(readonly target: WatchTarget) {
    // The working tree's subscription, and that of each folder or file named outside it.
    this.coverage = {
      covers: (path) => (this.live && within(path, target.root)) || this.covered.has(path) || this.covered.has(parentPath(path)),
    }
  }

  send(request: FileWatchRequest): void {
    this.link?.send(JSON.stringify(request))
  }
}

/**
 * A conversation's file watch (`web-api.md` § File watch): open while a
 * component shows the conversation's files, so the Host's reports of what
 * changed keep what the page read current (`plugin-pages.md` § What the
 * service keeps). It subscribes to the working tree and to the folders and
 * files shown outside it, and tells the Host's kept files what each `live`
 * covers, what a change concerns, and that what was covered is unconfirmed
 * when the Host cannot watch it or the watch ends. A socket that brings nothing, not
 * even a heartbeat, for 75 seconds is broken (`web-application.md`
 * § Liveness and reconnection); a closed or broken socket opens again after
 * the reconnect waits, and at once when the Host comes back online.
 */
export class ConversationWatch implements FileFollower {
  /** What the watch says, for the views: why the Host cannot watch, while it says so. */
  readonly note: { unavailable: string | null; refresh(): void }
  /** Each path shown, with how many views show it, the latest shown last. */
  private readonly shown = new Map<string, number>()
  private current: WatchSocket | null = null
  private waiting: ReconnectWait | null = null
  private failures = 0
  private sending = false

  constructor(
    private readonly conversationId: string,
    private readonly target: () => WatchTarget | null,
    private readonly openLink: OpenWatchLink = relayWatchLink,
  ) {
    this.note = reactive({
      unavailable: null as string | null,
      refresh: () => this.target()?.files.refresh(),
    })
  }

  show(path: string): () => void {
    const count = this.shown.get(path) ?? 0
    // The latest shown goes last, so the paths kept past the limit are the latest.
    this.shown.delete(path)
    this.shown.set(path, count + 1)
    if (!this.current && !this.waiting)
      this.connect()
    this.pathsChanged()
    let shown = true
    return () => {
      if (!shown)
        return
      shown = false
      const left = (this.shown.get(path) ?? 1) - 1
      if (left > 0)
        this.shown.set(path, left)
      else
        this.shown.delete(path)
      // A view that shows another entry lets the last one go first: the
      // watch closes only when nothing shows once the views have settled.
      queueMicrotask(() => {
        if (this.shown.size === 0)
          this.stop()
      })
      this.pathsChanged()
    }
  }

  /**
   * The Host came online, or became known: a watch that something shows
   * and that is not open, waiting to connect again or never connected,
   * connects now.
   */
  online(): void {
    if (this.current)
      return
    this.waiting?.cancel()
    this.waiting = null
    this.connect()
  }

  /**
   * The path to the Host changed (`direct-channel.md` § Choosing the
   * path): an open watch reopens on the other path at once, as after a lost
   * connection but without its wait, and what it covered is unconfirmed
   * until the new one says `live`.
   */
  move(): void {
    const watch = this.current
    if (!watch)
      return
    this.current = null
    this.end(watch)
    watch.link?.close()
    this.connect()
  }

  private connect(): void {
    const target = this.target()
    if (!target || this.shown.size === 0)
      return
    const watch = new WatchSocket(target)
    this.current = watch
    watch.link = this.openLink(this.conversationId, {
      opened: () => {
        if (this.current !== watch)
          return
        watch.silence = watchSilence('file watch', () => this.lose(watch))
        watch.opened = true
        watch.send({ type: 'subscribe', subscriptions: [{ key: TREE, kind: 'worktree', path: target.root }] })
        this.send(watch)
        watch.renewal = setInterval(() => watch.send({ type: 'renew', keys: [TREE, ...watch.held] }), RENEW_MS)
      },
      message: (data) => {
        if (this.current !== watch)
          return
        watch.silence?.heard()
        let value: unknown
        try {
          value = typeof data === 'string' ? JSON.parse(data) : null
        } catch {
          value = null
        }
        const parsed = fileWatchMessageSchema.safeParse(value)
        if (!parsed.success) {
          // What did not read is for a developer; the watch starts again.
          reportError('Could not read a message of the file watch.', z.prettifyError(parsed.error))
          watch.link?.close()
          this.lose(watch)
          return
        }
        this.receive(watch, parsed.data)
      },
      closed: () => this.lose(watch),
    })
  }

  /**
   * The socket closed, or is taken as broken: what it covered is
   * unconfirmed, and while something shows the files it connects again
   * after its wait.
   */
  private lose(watch: WatchSocket): void {
    if (this.current !== watch)
      return
    this.current = null
    this.end(watch)
    // A broken socket's own close may come much later, or never.
    watch.link?.close()
    if (this.shown.size === 0)
      return
    this.failures += 1
    this.waiting = waitToReconnect(this.failures, () => {
      this.waiting = null
      this.connect()
    })
  }

  private receive(watch: WatchSocket, message: FileWatchMessage): void {
    const { files, root } = watch.target
    switch (message.type) {
      case 'heartbeat':
      case 'renewed':
        return
      case 'live':
        if (message.key === TREE) {
          watch.live = true
          this.failures = 0
          this.note.unavailable = null
          files.cover(watch.coverage)
        } else if (watch.held.includes(message.key)) {
          watch.covered.add(message.key)
          files.cover(watch.coverage)
        }
        return
      case 'changed':
        // A key outside the tree is its path; the tree's names no path.
        files.changed(message.changes.map(([key]) => (key === TREE ? root : key)))
        return
      case 'unavailable':
        if (message.key === TREE) {
          this.note.unavailable = message.reason || NO_REASON
          // What the tree covered is unconfirmed; the paths outside it stay covered.
          files.uncover(watch.coverage)
          watch.live = false
          if (watch.covered.size > 0)
            files.cover(watch.coverage)
        } else {
          watch.covered.delete(message.key)
          watch.held = watch.held.filter((path) => path !== message.key)
          files.changed([message.key])
        }
        return
      case 'refused':
        watch.held = watch.held.filter((path) => path !== message.key)
        return
      case 'offline':
        // The socket closes after it.
        files.uncover(watch.coverage)
    }
  }

  /** The paths outside the working tree go to the watch, once the views have settled. */
  private pathsChanged(): void {
    if (this.sending)
      return
    this.sending = true
    queueMicrotask(() => {
      this.sending = false
      const watch = this.current
      if (watch?.opened)
        this.send(watch)
    })
  }

  /** Subscribes to the paths shown outside the working tree, and lets go of those no longer shown. */
  private send(watch: WatchSocket): void {
    const outside = [...this.shown.keys()].filter((path) => !within(path, watch.target.root)).slice(-MAX_OUTSIDE)
    const dropped = watch.held.filter((path) => !outside.includes(path))
    const added = outside.filter((path) => !watch.held.includes(path))
    watch.held = outside
    if (dropped.length > 0) {
      // A path no longer held is no longer covered, at once.
      for (const path of dropped)
        watch.covered.delete(path)
      watch.target.files.changed(dropped)
      watch.send({ type: 'unsubscribe', keys: dropped })
    }
    if (added.length > 0) {
      const subscriptions = added.map((path) => ({ key: path, kind: 'file' as const, path }))
      watch.send({ type: 'subscribe', subscriptions })
    }
  }

  /** What a socket covered is unconfirmed once it ends, and its silence is no longer watched. */
  private end(watch: WatchSocket): void {
    watch.silence?.stop()
    watch.silence = null
    if (watch.renewal !== null)
      clearInterval(watch.renewal)
    watch.renewal = null
    watch.target.files.uncover(watch.coverage)
    this.note.unavailable = null
  }

  /** No view shows the conversation's files: the watch closes. */
  private stop(): void {
    this.waiting?.cancel()
    this.waiting = null
    const watch = this.current
    this.current = null
    if (!watch)
      return
    this.end(watch)
    watch.link?.close()
  }
}
