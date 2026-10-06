import { reactive } from 'vue'
import { z } from 'zod'
import type { Coverage, HostFiles } from '@demicodes/web-ui/files/file-cache'
import type { FileFollower } from '@demicodes/web-ui/files/kept-source'
import { parentPath } from '@demicodes/web-ui/files/paths'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { waitToReconnect, type ReconnectWait } from '@demicodes/web-ui/transport/liveness'
import { apiUrl } from '../api/client'
import { fileWatchMessageSchema, type FileWatchMessage, type FileWatchRequest } from '../api/generated/web-api'

/** The most paths outside the working tree a watch names (`web-api.md` § File watch). */
const MAX_PATHS = 64

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

/**
 * One socket of the watch, from its open to its close: what it covers once
 * it is live, and how far its `paths` messages have been answered.
 */
class WatchSocket {
  readonly coverage: Coverage
  /** The paths outside the working tree the Host's watches run for. */
  covered = new Set<string>()
  /** The `paths` messages sent, and how many a `live` answered. */
  sent = 0
  answered = 0
  /** The paths of the last message sent. */
  last: string[] = []
  /** The first `live` came: the working tree is covered. */
  live = false
  /** A `lost` came, whose own `live` follows. */
  lost = false
  /** The Host cannot watch: no state follows on this socket. */
  unavailable = false

  constructor(readonly socket: WebSocket, readonly target: WatchTarget) {
    // The working tree's watch, which also follows its repository's `.git`,
    // and the watch of each folder or file named outside it.
    this.coverage = {
      covers: (path) => within(path, target.root) || this.covered.has(path) || this.covered.has(parentPath(path)),
    }
  }
}

/**
 * A conversation's file watch (`web-api.md` § File watch): open while a
 * component shows the conversation's files, so the Host's reports of what
 * changed keep what the page read current (`plugin-pages.md` § What the
 * service keeps). It tells the Host's kept files what each `live` covers,
 * what `changed` names, and that what was covered is unconfirmed when the
 * watch loses reports, cannot watch, or ends; and it names the folders and
 * files shown outside the working tree. A closed socket opens again after
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
      if (this.shown.size === 0)
        this.stop()
      else
        this.pathsChanged()
    }
  }

  /** The Host came back online: a watch waiting to connect again connects now. */
  online(): void {
    if (this.waiting) {
      this.waiting.cancel()
      this.waiting = null
      this.connect()
    }
  }

  private connect(): void {
    const target = this.target()
    if (!target || this.shown.size === 0)
      return
    const url = new URL(apiUrl(`/conversations/${encodeURIComponent(this.conversationId)}/fs/watch`), window.location.href)
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
    const watch = new WatchSocket(new WebSocket(url), target)
    this.current = watch
    watch.socket.addEventListener('message', (event) => {
      if (this.current !== watch)
        return
      let data: unknown
      try {
        data = typeof event.data === 'string' ? JSON.parse(event.data) : null
      } catch {
        data = null
      }
      const parsed = fileWatchMessageSchema.safeParse(data)
      if (!parsed.success) {
        // What did not read is for a developer; the watch starts again.
        reportError('Could not read a message of the file watch.', z.prettifyError(parsed.error))
        watch.socket.close()
        return
      }
      this.receive(watch, parsed.data)
    })
    watch.socket.addEventListener('close', () => {
      if (this.current !== watch)
        return
      this.end(watch)
      this.current = null
      if (this.shown.size === 0)
        return
      this.failures += 1
      this.waiting = waitToReconnect(this.failures, () => {
        this.waiting = null
        this.connect()
      })
    })
  }

  private receive(watch: WatchSocket, message: FileWatchMessage): void {
    const { files } = watch.target
    if (message.type === 'changed') {
      files.changed(message.paths)
      return
    }
    switch (message.state) {
      case 'live':
        if (!watch.live) {
          // The first: the working tree is covered, and the paths outside it follow.
          watch.live = true
          this.failures = 0
          this.note.unavailable = null
          files.cover(watch.coverage)
          this.pathsChanged()
        } else if (watch.lost) {
          watch.lost = false
          files.cover(watch.coverage)
        } else {
          watch.answered += 1
          // Every message answered: each path the last one names runs.
          if (watch.answered === watch.sent)
            watch.covered = new Set(watch.last)
        }
        return
      case 'lost':
        watch.lost = true
        files.uncover(watch.coverage, true)
        return
      case 'unavailable':
        watch.unavailable = true
        this.note.unavailable = message.reason ?? NO_REASON
        files.uncover(watch.coverage)
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
      // A socket that brought its first `live` is open.
      if (watch?.live && !watch.unavailable)
        this.send(watch)
    })
  }

  private send(watch: WatchSocket): void {
    const outside = [...this.shown.keys()].filter((path) => !within(path, watch.target.root))
    const paths = outside.slice(-MAX_PATHS)
    if (paths.length === watch.last.length && paths.every((path, index) => watch.last[index] === path))
      return
    // A path no longer named is no longer covered, at once; one newly named is once a `live` answers.
    const dropped = [...watch.covered].filter((path) => !paths.includes(path))
    for (const path of dropped)
      watch.covered.delete(path)
    if (dropped.length > 0)
      watch.target.files.changed(dropped)
    const request: FileWatchRequest = { type: 'paths', paths }
    watch.socket.send(JSON.stringify(request))
    watch.sent += 1
    watch.last = paths
  }

  /** What a socket covered is unconfirmed once it ends. */
  private end(watch: WatchSocket): void {
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
    watch.socket.close(1000)
  }
}
