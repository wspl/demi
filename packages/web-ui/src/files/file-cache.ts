/**
 * What a page keeps of one Host's files (`plugin-pages.md` § What the
 * service keeps): listings, texts, descriptions, the changes list and each
 * changed file's sides, each the last answer read, by the absolute path on
 * the Host. Every entry kept holds the subscriptions it depends on, carried
 * by the watch of the view that brought it, until it leaves the cache; only
 * what a view shows on screen is read.
 *
 * Whether an entry is current is worked out each time from numbers, never
 * stored: its answer's number against each subscription's `since`, the
 * `at` of its last change, its epoch and its lease. A view that shows an
 * entry that is not current shows what is kept at once and reads it again
 * in the background; a change of an entry on screen reads it again, a
 * folder's listing and a changes list at most once a second; a change of
 * one off screen is only noted. An entry holds no way to read it: each view
 * reads through its own route, and a read nobody asked for goes through the
 * route of the view that came on screen last. Reads of one entry share one
 * request. At most `budget` of text and `MAX_ENTRIES` entries stay, the
 * entries shown longest ago leaving first.
 */
import { requestIdOf } from '../infra/errors'
import { reactive, shallowRef } from 'vue'
import { FileBrowserError, type FileBrowserFailure } from './types'
import {
  Subscription,
  type ReadAt,
  type Subject,
  type WatchChannel,
  type WatchListener,
} from './watch-channel'

/** The most text a page keeps of the Hosts' files, counted in characters. */
export const KEPT_TEXT = 64 * 1024 * 1024

/** The most entries a page keeps of one Host's files, as many as the subscriptions it may hold (`web-api.md` § File watch). */
export const MAX_ENTRIES = 4096

/**
 * The least time between two reads of a folder's listing or a working
 * tree's changes list that changes ask for, in milliseconds: a log appended
 * many times a second updates its size once a second, as Finder does, and a
 * build that writes files all the time lists the repository's changes once
 * a second, as an editor's source control view refreshes, rather than after
 * every batch of changes, each a status over the whole repository.
 */
export const SUMMARY_REREAD_MS = 1000

/** What a view says of an answer that nothing is at the path. */
export const GONE_MESSAGE = 'It is no longer there.'

/**
 * What an entry is, which decides what it subscribes to (`plugin-pages.md`
 * § What the service keeps): a file's text or description its `file`, a
 * listing its folder's names or details, the changes list its working tree,
 * a changed file's sides its `file` and its repository's `head`, and its
 * committed contents the `head` alone; an `answer` of the backend that
 * reads several files, such as a project's workflows, what its last answer
 * names (`KeptSpec.subjects`).
 */
export type KeptKind = 'listing' | 'text' | 'description' | 'changes' | 'sides' | 'committed' | 'answer'

/**
 * What the last answer was: a value; nothing at the path; a file that is
 * not text; or one too large to read as text. Each is kept, carries its
 * version and number, and is current or not alike.
 */
export type KeptAnswer = 'value' | 'gone' | 'binary' | 'too-large'

/** One kept answer as a view shows it; reactive. */
export interface ShownEntry<T> {
  /** The last answer's value; undefined before the first answer, and while the last holds none. */
  readonly value: T | undefined
  /** What the last answer was; null before the first. A view draws each as itself, never as a failure. */
  readonly answer: KeptAnswer | null
  /**
   * Why the last read could not be made, a Host out of reach or a refusal.
   * Without an answer it is what the view shows, as a first read would;
   * beside one the view keeps the answer and says quietly that it could not
   * refresh.
   */
  readonly failure: FileBrowserFailure | null
  /** A read is on its way: the first, which a view shows as loading, or a check, which it does not. */
  readonly reading: boolean
  /**
   * Why the Host does not watch it, while a subscription it depends on is
   * refused or unavailable: the view says quietly that it shows what was
   * last read, with Refresh, and it is read each time it is shown.
   */
  readonly unwatched: string | null
}

/** What a view without a value shows in its place: that the path is gone, or why the read could not be made. */
export function failureShown(entry: ShownEntry<unknown> | null | undefined): FileBrowserFailure | null {
  if (!entry || entry.answer === 'value')
    return null
  if (entry.answer === 'gone')
    return { kind: 'not-found', message: GONE_MESSAGE }
  return entry.failure
}

/** A view's hold on an entry it shows. */
export interface Showing<T> {
  readonly entry: ShownEntry<T>
  /**
   * Reads the entry again now, as Retry and Refresh do, or joins the read on
   * its way. It settles when that read ends, whatever it answered, so a
   * region's Retry stops loading even when the answer is the one it showed
   * before, such as a file that is still not there (`RegionStatus`).
   */
  retry(): Promise<void>
  /** The view no longer shows it; a second release does nothing. */
  release(): void
  /**
   * The view holds it but is not on screen, as a work panel tab the user
   * switched away from: changes are only noted (`plugin-pages.md` § What
   * the service keeps). A second call does nothing.
   */
  away(): void
  /** The view is on screen again: an entry that is not current is read once. */
  back(): void
}

/**
 * What a read answered, with the Host's number before it read; null for a
 * source without numbers, whose answers are never current.
 */
export interface Answered<T> {
  value: T
  readAt: ReadAt | null
}

/**
 * What a read that names the version kept answered: a value, or that the
 * version is still the Host's, so what is kept is.
 */
export type Read<T> = Answered<T> | { unchanged: true; readAt: ReadAt | null }

/** How an entry is read, measured and watched. */
export interface KeptSpec<T> {
  kind: KeptKind
  /** The absolute path on the Host: a listing's folder, a file, the changes list's root. */
  path: string
  /** Tells the entries of one path and kind apart, such as a changed file's sides from its working tree root. */
  key?: string
  /** For a changed file's sides and its committed contents: the working tree's root, whose `head` they depend on. */
  root?: string
  /** For a listing: the view shows its entries' sizes and times, so it follows them too. */
  details?: boolean
  /** Reads it; `held` is the version kept, which a read may answer unchanged with. */
  read(held: string | null): Promise<Read<T>>
  /** The characters of text it holds. */
  size(value: T): number
  /** The version of `value`, which the next read names; none without versions. */
  version?(value: T): string | null
  /** The watch of the view's route, which carries the subscriptions the entry takes; none without one. */
  watch?: WatchChannel | null
  /** For an `answer`: what the answer `value` depends on, none before the first. */
  subjects?(value: T | undefined): Subject[]
  /**
   * Its value never changes once read, such as a file at a commit's full
   * id: it follows nothing and is current from then on.
   */
  immutable?: boolean
}

/** How a view reads an entry: through the route of the place that shows it. */
type ReadEntry<T> = KeptSpec<T>['read']

/** A view's hold on an entry, with the way that view reads and watches it. */
interface Holder<T> {
  read: ReadEntry<T>
  watch: WatchChannel | null
  details: boolean
  /** The view holds it but is not on screen. */
  away: boolean
}

/**
 * Whether a kept answer holds now: `current`; `stale` when the numbers say
 * it changed after the read; `restarted` when the answer and the watch are
 * of different runs of the runner, which one more read settles once both
 * are of the same; or `unknown` when no watch says.
 */
type Freshness = 'current' | 'stale' | 'restarted' | 'unknown'

/** The answers a read refused with that are answers, not failures. */
function answerOf(failure: FileBrowserFailure['kind']): KeptAnswer | null {
  switch (failure) {
    case 'not-found':
      return 'gone'
    case 'binary':
      return 'binary'
    case 'too-large':
      return 'too-large'
    default:
      return null
  }
}

function failureOf(error: unknown): FileBrowserFailure {
  const requestId = requestIdOf(error)
  if (error instanceof FileBrowserError)
    return { kind: error.kind, message: error.message === error.kind ? undefined : error.message, requestId }
  return { kind: 'other', message: error instanceof Error ? error.message : String(error), requestId }
}

/** The subscription key of `subject`, by which the kept files share it. */
function subjectKey({ kind, path }: Subject): string {
  return `${kind}\u0000${path}`
}

/**
 * One kept answer, shared by every view of its Host and path: data alone,
 * what was read, its number and the subscriptions it holds, never a way to
 * read it. Each view reads it through its own route, as a conversation's
 * views through the conversation's and a device's folder dialog through the
 * device's (`sessions-and-targets.md` § Every way to a Host), and every view
 * sees what one of them read.
 */
class Entry<T> {
  readonly spec: Omit<KeptSpec<T>, 'read' | 'watch' | 'details'>
  /** The subscriptions it holds while kept; none until a view shows or reads it. */
  readonly subscriptions = shallowRef<readonly Subscription[]>([])
  readonly state: { value: T | undefined; answer: KeptAnswer | null; failure: FileBrowserFailure | null; reading: boolean; readonly unwatched: string | null }
  /** The views that hold it, the one that came on screen last at the end. */
  readonly holders = new Set<Holder<T>>()
  /** The last answer's version and number. */
  version: string | null = null
  readAt: ReadAt | null = null
  /** A listing follows its entries' sizes and times. */
  details = false
  /** When it was last shown, by the cache's clock. */
  lastShown = 0
  size = 0
  reading: Promise<void> | null = null
  /** When its last read began, by `performance.now()`. */
  readStart = Number.NEGATIVE_INFINITY
  /** The read a change asked for that waits for the listing's second to end. */
  rereadTimer: ReturnType<typeof setTimeout> | null = null

  // The way to read and watch belongs to the view that brought the spec; the entry keeps none.
  constructor({ read: _read, watch: _watch, details: _details, ...spec }: KeptSpec<T>) {
    this.spec = spec
    const subscriptions = this.subscriptions
    this.state = reactive({
      value: undefined,
      answer: null,
      failure: null,
      reading: false,
      get unwatched() {
        for (const subscription of subscriptions.value) {
          const { status, reason } = subscription.state
          if (status === 'unavailable' || status === 'refused')
            return reason
        }
        return null
      },
    }) as Entry<T>['state']
  }

  /** How many views show it on screen. */
  get shown(): number {
    let count = 0
    for (const holder of this.holders) {
      if (!holder.away)
        count += 1
    }
    return count
  }

  /** The view that came on screen last; null while none shows it. */
  latest(): Holder<T> | null {
    let latest: Holder<T> | null = null
    for (const holder of this.holders) {
      if (!holder.away)
        latest = holder
    }
    return latest
  }

  /** How many times its subscriptions went live, all told. */
  lives(): number {
    return this.subscriptions.value.reduce((sum, subscription) => sum + subscription.lives, 0)
  }

  /** What it subscribes to: the subjects its answer depends on. */
  subjects(): Subject[] {
    const { kind, path, root } = this.spec
    switch (kind) {
      case 'text':
      case 'description':
        return [{ kind: 'file', path }]
      case 'listing':
        return [{ kind: this.details ? 'folder_details' : 'folder_names', path }]
      case 'changes':
        return [{ kind: 'worktree', path }]
      case 'sides':
        return root === undefined ? [{ kind: 'file', path }] : [{ kind: 'file', path }, { kind: 'head', path: root }]
      case 'committed':
        return root === undefined ? [] : [{ kind: 'head', path: root }]
      case 'answer':
        return this.spec.subjects?.(this.state.value) ?? []
    }
  }

  /**
   * Whether its answer holds now (`plugin-pages.md` § What the service
   * keeps): current when every subscription it holds is live within its
   * lease as the page counts it, of the answer's epoch, started at or before
   * the answer's number and changed last at or before it; stale when the
   * subscriptions are live but the numbers say a change came after the
   * read; restarted when they are live in another run than the answer's;
   * unknown otherwise, which no change will settle.
   */
  freshness(now: number): Freshness {
    if (this.spec.immutable && this.state.answer === 'value')
      return 'current'
    const readAt = this.readAt
    const subscriptions = this.subscriptions.value
    if (this.state.answer === null || readAt === null || subscriptions.length === 0)
      return 'unknown'
    let freshness: Freshness = 'current'
    for (const subscription of subscriptions) {
      if (!subscription.watching(now))
        return 'unknown'
      if (subscription.epoch !== readAt.epoch)
        freshness = 'restarted'
      else if (freshness === 'current' && (subscription.since > readAt.number || subscription.changedAt > readAt.number))
        freshness = 'stale'
    }
    return freshness
  }
}

/** The kept files of one Host, which every conversation and page on it shares. */
export class HostFiles implements WatchListener {
  private readonly entries = new Map<string, Entry<unknown>>()
  /** The subscriptions the kept entries hold, each subject once. */
  private readonly subscriptions = new Map<string, Subscription>()
  private clock = 0
  private kept = 0
  private keys = 0

  constructor(private readonly budget = KEPT_TEXT, private readonly maxEntries = MAX_ENTRIES) {}

  private entry<T>(spec: KeptSpec<T>): Entry<T> {
    const key = `${spec.kind}\u0000${spec.path}\u0000${spec.key ?? ''}`
    let entry = this.entries.get(key) as Entry<T> | undefined
    if (!entry) {
      entry = new Entry(spec)
      this.entries.set(key, entry as Entry<unknown>)
    }
    return entry
  }

  /**
   * Has `entry` hold the subscriptions it depends on now: a listing that a
   * view of sizes and times holds follows its details, else what it
   * followed. A subscription not watched where it is, such as after its
   * watch closed, moves to the watch of the view that came last.
   */
  private subscribe(entry: Entry<unknown>, given: WatchChannel | null): void {
    if (entry.spec.kind === 'listing' && entry.holders.size > 0)
      entry.details = [...entry.holders].some((holder) => holder.details)
    // Without a view's watch of its own, as when a view lets go, the one of
    // the view that came last carries what the entry takes up.
    const watch = given ?? [...entry.holders].reverse().find((holder) => holder.watch)?.watch ?? null
    const held = entry.subscriptions.value
    const subjects = entry.subjects()
    const wanted = subjects.map((subject) => subjectKey(subject))
    const same = held.length === wanted.length &&
      held.every((subscription, index) => subjectKey(subscription.subject) === wanted[index])
    if (!same) {
      entry.subscriptions.value = subjects.map((subject, index) => {
        let subscription = this.subscriptions.get(wanted[index]!)
        if (!subscription) {
          this.keys += 1
          subscription = new Subscription(`s${this.keys}`, subject)
          this.subscriptions.set(wanted[index]!, subscription)
        }
        subscription.holders += 1
        return subscription
      })
      this.unsubscribe(held)
    }
    if (!watch)
      return
    for (const subscription of entry.subscriptions.value) {
      if (subscription.channel !== watch && subscription.state.status !== 'live') {
        subscription.channel?.drop(subscription)
        watch.carry(subscription)
      }
    }
  }

  /** Lets go of `subscriptions` for an entry: each that no other entry holds is unsubscribed at once. */
  private unsubscribe(subscriptions: readonly Subscription[]): void {
    for (const subscription of subscriptions) {
      subscription.holders -= 1
      if (subscription.holders > 0)
        continue
      this.subscriptions.delete(subjectKey(subscription.subject))
      subscription.channel?.drop(subscription)
    }
  }

  /**
   * Reads `entry` through `read`, or joins the read on its way, whatever
   * route that one took: what it answers is the Host's. An answer that
   * the numbers already show changed reads once more for the views that
   * show it.
   */
  private read<T>(entry: Entry<T>, read: ReadEntry<T>): Promise<void> {
    if (entry.reading)
      return entry.reading
    // This read sees every change so far, so a reread still waiting is not needed.
    this.cancelReread(entry)
    entry.readStart = performance.now()
    const lives = entry.lives()
    entry.state.reading = true
    // A view with nothing to show but a failure shows the read instead, as
    // a region's Retry returns it to loading (`RegionStatus`); an answer
    // stays shown while it is checked.
    if (entry.state.answer === null)
      entry.state.failure = null
    const reading = (async () => {
      let answered = false
      try {
        const answer = await read(entry.version)
        entry.readAt = answer.readAt
        // Unchanged: the answer kept is the Host's, and nothing a view shows changes.
        if (!('unchanged' in answer)) {
          const value = answer.value
          if (value !== entry.state.value)
            entry.state.value = value
          entry.state.answer = 'value'
          entry.version = entry.spec.version?.(value) ?? null
          this.measure(entry, entry.spec.size(value))
          // An answer names what it depends on: the entry follows that now.
          if (entry.spec.kind === 'answer')
            this.subscribe(entry as Entry<unknown>, null)
        }
        entry.state.failure = null
        answered = true
      } catch (error) {
        const failure = failureOf(error)
        const kind = answerOf(failure.kind)
        if (kind !== null && error instanceof FileBrowserError) {
          // An answer like any other, as a first read's would be.
          entry.state.value = undefined
          entry.state.answer = kind
          entry.state.failure = null
          entry.version = error.version
          entry.readAt = error.readAt
          this.measure(entry, 0)
          answered = true
        } else {
          // A read that could not be made keeps what the view shows.
          entry.state.failure = failure
        }
      } finally {
        entry.reading = null
        entry.state.reading = false
      }
      // The numbers say a change came after this read, or a watch of another
      // run went live while it was on its way: it is read once more. An
      // answer of a run the watch has not gone live in yet waits for that.
      const freshness = answered && entry.shown > 0 ? entry.freshness(Date.now()) : 'unknown'
      if (freshness === 'stale' || (freshness === 'restarted' && entry.lives() !== lives))
        this.reread(entry)
      this.evict()
    })()
    entry.reading = reading
    return reading
  }

  /**
   * Reads again an entry a view shows, for a change that named it: at once,
   * but a folder's listing or a changes list at most once a second. A change
   * within a second of its last read waits for that second to end and is
   * then read with any that came meanwhile, so the last change is never
   * dropped.
   */
  private reread(entry: Entry<unknown>): void {
    if (entry.spec.kind !== 'listing' && entry.spec.kind !== 'changes') {
      this.readShown(entry)
      return
    }
    if (entry.rereadTimer !== null)
      return
    const wait = entry.readStart + SUMMARY_REREAD_MS - performance.now()
    if (wait <= 0) {
      this.readShown(entry)
      return
    }
    entry.rereadTimer = setTimeout(() => {
      entry.rereadTimer = null
      // A view that let go meanwhile reads it when it shows it next, as it is out of date.
      this.readShown(entry)
    }, wait)
  }

  /** Reads an entry a view shows on screen again, through that view's route; one none shows is left for the next view to show it. */
  private readShown(entry: Entry<unknown>): void {
    const holder = entry.latest()
    if (holder)
      void this.read(entry, holder.read)
  }

  private cancelReread(entry: Entry<unknown>): void {
    if (entry.rereadTimer === null)
      return
    clearTimeout(entry.rereadTimer)
    entry.rereadTimer = null
  }

  private measure(entry: Entry<unknown>, size: number): void {
    this.kept += size - entry.size
    entry.size = size
  }

  /**
   * Lets go of the entries shown longest ago, none a view holds or being
   * read, until what is kept fits the budget and the count, each giving up
   * its subscriptions at once.
   */
  private evict(): void {
    if (this.kept <= this.budget && this.entries.size <= this.maxEntries)
      return
    const idle = [...this.entries.entries()]
      .filter(([, entry]) => entry.holders.size === 0 && !entry.reading)
      .sort(([, a], [, b]) => a.lastShown - b.lastShown)
    for (const [key, entry] of idle) {
      if (this.kept <= this.budget && this.entries.size <= this.maxEntries)
        return
      this.kept -= entry.size
      this.cancelReread(entry)
      this.entries.delete(key)
      this.unsubscribe(entry.subscriptions.value)
      entry.subscriptions.value = []
    }
  }

  /**
   * Makes sure `entry` holds what it depends on through `watch`, and
   * subscribes again to what the page counts as lapsed: a view that shows
   * or reads it holds it current from now on.
   */
  private follow(entry: Entry<unknown>, watch: WatchChannel | null): void {
    this.subscribe(entry, watch)
    const now = Date.now()
    for (const subscription of entry.subscriptions.value)
      subscription.channel?.renewLapsed(subscription, now)
  }

  /**
   * A view shows the entry `spec` names until it releases it: what is kept
   * shows at once, and an entry that is not current is read.
   */
  show<T>(spec: KeptSpec<T>): Showing<T> {
    const entry = this.entry(spec)
    const holder: Holder<T> = { read: spec.read, watch: spec.watch ?? null, details: spec.details ?? false, away: false }
    entry.holders.add(holder)
    entry.lastShown = ++this.clock
    this.follow(entry as Entry<unknown>, holder.watch)
    if (entry.freshness(Date.now()) !== 'current')
      void this.read(entry, spec.read)
    this.evict()
    let released = false
    return {
      entry: entry.state,
      retry: () => this.read(entry, spec.read),
      release: () => {
        if (released)
          return
        released = true
        entry.holders.delete(holder)
        entry.lastShown = ++this.clock
        // A listing the views that remain show without sizes follows the names again.
        if (entry.holders.size > 0)
          this.subscribe(entry as Entry<unknown>, null)
        this.evict()
      },
      away: () => {
        if (released || holder.away)
          return
        holder.away = true
        entry.lastShown = ++this.clock
      },
      back: () => {
        if (released || !holder.away)
          return
        holder.away = false
        // Last on screen: its route reads the entry again for changes.
        entry.holders.delete(holder)
        entry.holders.add(holder)
        entry.lastShown = ++this.clock
        this.follow(entry as Entry<unknown>, holder.watch)
        if (entry.freshness(Date.now()) !== 'current')
          void this.read(entry, spec.read)
      },
    }
  }

  /**
   * Edits what is kept of the entry `spec` names, as the page's own write
   * tells it, without a read: an upload's new file in its folder's listing.
   * Nothing kept, nothing to edit; whether it is current stays as it was,
   * since the Host's watch reports the write too.
   */
  amend<T>(spec: KeptSpec<T>, edit: (value: T) => T): void {
    const entry = this.entries.get(`${spec.kind}\u0000${spec.path}\u0000${spec.key ?? ''}`) as Entry<T> | undefined
    const value = entry?.state.value
    if (!entry || value === undefined)
      return
    const edited = edit(value)
    entry.state.value = edited
    this.measure(entry as Entry<unknown>, spec.size(edited))
  }

  /** Reads the entry `spec` names again now, as a Refresh does; it settles when that read ends. */
  retry<T>(spec: KeptSpec<T>): Promise<void> {
    const entry = this.entry(spec)
    this.follow(entry as Entry<unknown>, spec.watch ?? null)
    return this.read(entry, spec.read)
  }

  /** What is kept of the entry `spec` names, as a view would show it, without showing or reading it. */
  peek<T>(spec: KeptSpec<T>): ShownEntry<T> {
    return this.entry(spec).state
  }

  /** The entry once, for work that shows nothing: the current answer kept, or a read's. */
  async get<T>(spec: KeptSpec<T>): Promise<T> {
    const entry = this.entry(spec)
    entry.lastShown = ++this.clock
    this.follow(entry as Entry<unknown>, spec.watch ?? null)
    if (entry.freshness(Date.now()) !== 'current')
      await this.read(entry, spec.read)
    const { value, answer, failure } = entry.state
    if (answer === 'value' && value !== undefined)
      return value
    if (answer === 'gone')
      throw new FileBrowserError('not-found', GONE_MESSAGE)
    if (answer === 'binary' || answer === 'too-large')
      throw new FileBrowserError(answer)
    throw new FileBrowserError(failure?.kind ?? 'other', failure?.message)
  }

  /**
   * The Host said something of `subscriptions`: each entry on screen that
   * depends on one and whose answer the numbers now show changed is read
   * again, at once but a listing and a changes list at most once a second,
   * and so is one whose read failed, as a history read outside a repository
   * does until `git init` makes one there. An entry off screen is only
   * noted, by the numbers its subscriptions now hold, and read when it
   * comes on screen.
   */
  heard(subscriptions: readonly Subscription[]): void {
    const heard = new Set(subscriptions)
    const now = Date.now()
    for (const entry of this.entries.values()) {
      if (entry.shown === 0 || entry.reading || !entry.subscriptions.value.some((subscription) => heard.has(subscription)))
        continue
      const failed = entry.state.answer === null && entry.state.failure !== null
      const freshness = entry.freshness(now)
      if (failed || freshness === 'stale' || freshness === 'restarted')
        this.reread(entry)
    }
  }
}
