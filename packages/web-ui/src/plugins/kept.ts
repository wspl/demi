/**
 * Sessions by key that outlive the views that watch them
 * (`decisions/0008-plugin-architecture.md`), such as a device's live view or a
 * conversation's browser.
 *
 * A view watches a session while it shows it and lets go when it stops; what
 * only reads a session, such as a tab's title, reaches it without watching. A
 * session nobody watches is kept, with its connection and what it last
 * showed, so the next view shows that at once; past `limit` of them, the one
 * watched least recently is released. A watched session is never released
 * for the limit.
 *
 * ```text
 * watch(k) ─▶ watched ──unwatch──▶ kept ──over the limit──▶ released
 *                ▲                   │
 *                └──────watch(k)─────┘
 * ```
 */
import { shallowRef } from 'vue'

/** What a session does as views watch it and as it ends. */
export interface KeptSession {
  /** Someone watches it now, or nobody does: an unwatched session stops its costly work and keeps what it shows. */
  watched(watched: boolean): void
  /** It is released: everything it holds ends. */
  dispose(): void
}

/** How many sessions nobody watches a plugin keeps by default. */
export const KEPT_SESSIONS = 16

interface Entry<S> {
  session: S
  watchers: number
}

/** One view's watch of a session; `end` lets it go. */
export interface SessionWatch<S> {
  readonly session: S
  end(): void
}

export class KeptSessions<S extends KeptSession> {
  /** Every session, the least recently watched first. */
  private readonly entries = new Map<string, Entry<S>>()
  /**
   * Counts the releases, which a reader of `get` and `of` follows. Making a
   * session counts nothing, so a component's render may make one through
   * `of` without changing what it reads.
   */
  private readonly released = shallowRef(0)

  constructor(
    private readonly make: (key: string) => S,
    private readonly limit = KEPT_SESSIONS,
  ) {}

  /** The session of `key`, if one is kept or watched; read reactively. */
  get(key: string): S | undefined {
    void this.released.value
    return this.entries.get(key)?.session
  }

  /**
   * The session of `key`, made unwatched when there is none, as a tab's title
   * reads a conversation's browser; read reactively. Making one may let the
   * least recently watched go past the limit, after the caller's task.
   */
  of(key: string): S {
    void this.released.value
    const found = this.entries.get(key)
    if (found) {
      return found.session
    }
    const entry = { session: this.make(key), watchers: 0 }
    this.entries.set(key, entry)
    queueMicrotask(() => this.trim())
    return entry.session
  }

  /** The keys of every session, the least recently watched first. */
  keys(): string[] {
    return [...this.entries.keys()]
  }

  /** Watches the session of `key`, made now when there is none, until the answer's `end`. */
  watch(key: string): SessionWatch<S> {
    let entry = this.entries.get(key)
    if (entry) {
      // Watched now: the most recent.
      this.entries.delete(key)
    } else {
      entry = { session: this.make(key), watchers: 0 }
    }
    this.entries.set(key, entry)
    entry.watchers += 1
    if (entry.watchers === 1) {
      entry.session.watched(true)
    }
    let ended = false
    const watched = entry
    return {
      session: entry.session,
      end: () => {
        if (ended) {
          return
        }
        ended = true
        watched.watchers -= 1
        if (watched.watchers === 0 && this.entries.get(key) === watched) {
          watched.session.watched(false)
          this.trim()
        }
      },
    }
  }

  /** Releases the session of `key` now, watched or not, as when what it shows is gone. */
  release(key: string): void {
    const entry = this.entries.get(key)
    if (!entry) {
      return
    }
    this.entries.delete(key)
    this.released.value += 1
    entry.session.dispose()
  }

  /** Releases every session. */
  dispose(): void {
    const sessions = [...this.entries.values()].map((entry) => entry.session)
    this.entries.clear()
    this.released.value += 1
    for (const session of sessions) {
      session.dispose()
    }
  }

  /** Releases the least recently watched of the unwatched sessions past the limit. */
  private trim(): void {
    const unwatched = [...this.entries].filter(([, entry]) => entry.watchers === 0)
    for (const [key] of unwatched.slice(0, Math.max(0, unwatched.length - this.limit))) {
      this.release(key)
    }
  }
}
