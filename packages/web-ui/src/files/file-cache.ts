/**
 * What a page keeps of one Host's files (`plugin-pages.md` § What the
 * service keeps): listings, texts, descriptions, the changes list and each
 * changed file's sides, each the last answer read, by the absolute path on
 * the Host. An entry read while a live watch covered its path is confirmed:
 * showing it again asks nothing until a report of that watch names its path.
 * Any other entry is unconfirmed: showing it shows it at once and reads it
 * again. Reads of one entry share one request, and a report that names an
 * entry being read reads it once more after. A report reads a shown entry
 * again at once, a folder's listing at most once a second. At most `budget`
 * of text stay, the entries shown longest ago leaving first.
 */
import { reactive } from 'vue'
import { parentPath } from './paths'
import { FileBrowserError, type FileBrowserFailure } from './types'

/** The most text a page keeps of the Hosts' files, counted in characters. */
export const KEPT_TEXT = 64 * 1024 * 1024

/**
 * The least time between two reads of a folder's listing that reports ask
 * for, in milliseconds: a log appended many times a second updates its
 * size once a second, as Finder does.
 */
export const LISTING_REREAD_MS = 1000

/**
 * What an entry is, which decides which reports concern it: a listing is
 * its folder's, and a file's entries are the file's; the changes list and
 * the committed sides follow the repository's `.git` too, and a changes
 * list outside any repository follows only a `.git` that appears in it.
 */
export type KeptKind = 'listing' | 'text' | 'description' | 'changes' | 'sides' | 'committed'

/** One kept answer as a view shows it; reactive. */
export interface ShownEntry<T> {
  /** The last answer read; undefined before the first, and once a read found the path gone. */
  readonly value: T | undefined
  /**
   * Why the last read failed. Without a value it is what the view shows, as
   * a first read would; beside a value the view keeps the value and says
   * quietly that it could not refresh.
   */
  readonly failure: FileBrowserFailure | null
  /** A read is on its way: the first, which a view shows as loading, or a check, which it does not. */
  readonly reading: boolean
}

/** A view's hold on an entry it shows. */
export interface Showing<T> {
  readonly entry: ShownEntry<T>
  /** Reads the entry again now, as Retry does. */
  retry(): void
  /** The view no longer shows it; a second release does nothing. */
  release(): void
}

/** What a live watch of the Host covers: a report of it names every change at such a path. */
export interface Coverage {
  covers(path: string): boolean
}

/** How an entry is read and measured. */
export interface KeptSpec<T> {
  kind: KeptKind
  /** The absolute path on the Host whose reports concern it: a listing's folder, a file, the changes list's root. */
  path: string
  /** Tells the entries of one path and kind apart, such as a changed file's sides from its working tree root. */
  key?: string
  /** Reads it; `held` is the last answer kept, which a read may answer unchanged with. */
  read(held: T | undefined): Promise<T>
  /** The characters of text it holds. */
  size(value: T): number
  /**
   * Whether `value` is a changes list that found its path outside any git
   * repository: it changes only once a `.git` appears there, so no other
   * report concerns it, however often the files there change.
   */
  outsideRepository?(value: T): boolean
}

/** Whether `path` lies in a repository's `.git`, whose changes concern the changes list and the committed sides. */
export function inGitDirectory(path: string): boolean {
  return /(^|\/)\.git(\/|$)/.test(path.replaceAll('\\', '/'))
}

/** Whether `path` is `ancestor` or lies below it. */
function within(path: string, ancestor: string): boolean {
  const prefix = ancestor.endsWith('/') ? ancestor : `${ancestor}/`
  return path === ancestor || path.startsWith(prefix)
}

class Entry<T> {
  readonly state: ShownEntry<T> & { value: T | undefined; failure: FileBrowserFailure | null; reading: boolean }
  confirmed = false
  /** How many views show it. */
  shown = 0
  /** When it was last shown, by the cache's clock. */
  lastShown = 0
  size = 0
  /** Rises with each report that names it: a read that began before one cannot confirm it. */
  stamp = 0
  reading: Promise<void> | null = null
  /** When its last read began, by `performance.now()`. */
  readStart = Number.NEGATIVE_INFINITY
  /** The read a report asked for that waits for the listing's second to end. */
  rereadTimer: ReturnType<typeof setTimeout> | null = null

  constructor(readonly spec: KeptSpec<T>) {
    this.state = reactive({ value: undefined, failure: null, reading: false }) as Entry<T>['state']
  }

  /**
   * Whether a report of a change at `path` concerns it; `ignored` when git
   * ignores the path, which then changes no working tree's changes.
   */
  concerns(path: string, ignored: boolean): boolean {
    const own = this.spec.path
    switch (this.spec.kind) {
      case 'listing':
        // Its own folder, a folder above it, or an entry of it came, went or was renamed.
        return within(own, path) || parentPath(path) === own
      case 'text':
      case 'description':
        return within(own, path)
      case 'sides':
        return within(own, path) || inGitDirectory(path)
      case 'changes': {
        const value = this.state.value
        if (value !== undefined && this.spec.outsideRepository?.(value))
          return within(path, `${own.replace(/\/$/, '')}/.git`)
        return (within(path, own) && !ignored) || inGitDirectory(path)
      }
      case 'committed':
        return inGitDirectory(path)
    }
  }
}

function failureOf(error: unknown): FileBrowserFailure {
  if (error instanceof FileBrowserError)
    return { kind: error.kind, message: error.message === error.kind ? undefined : error.message }
  return { kind: 'other', message: error instanceof Error ? error.message : String(error) }
}

/** The kept files of one Host, which every conversation and page on it shares. */
export class HostFiles {
  private readonly entries = new Map<string, Entry<unknown>>()
  private readonly coverages = new Set<Coverage>()
  private clock = 0
  private kept = 0

  constructor(private readonly budget = KEPT_TEXT) {}

  private entry<T>(spec: KeptSpec<T>): Entry<T> {
    const key = `${spec.kind}\u0000${spec.path}\u0000${spec.key ?? ''}`
    let entry = this.entries.get(key) as Entry<T> | undefined
    if (!entry) {
      entry = new Entry(spec)
      this.entries.set(key, entry as Entry<unknown>)
    }
    return entry
  }

  /** The live coverage of `path`, if any. */
  private coverageOf(path: string): Coverage | null {
    for (const coverage of this.coverages) {
      if (coverage.covers(path))
        return coverage
    }
    return null
  }

  /**
   * Reads `entry`, or joins the read on its way. The answer confirms it
   * when a live watch covered its path from the read's start to its end and
   * no report named it meanwhile; a report that did reads it once more for
   * the views that show it.
   */
  private read<T>(entry: Entry<T>): Promise<void> {
    if (entry.reading)
      return entry.reading
    const stamp = entry.stamp
    const coverage = this.coverageOf(entry.spec.path)
    // This read sees every report so far, so a reread still waiting is not needed.
    this.cancelReread(entry)
    entry.readStart = performance.now()
    entry.state.reading = true
    // A view with nothing to show but the failure shows the read instead, as
    // a region's Retry returns it to loading (`RegionStatus`).
    if (entry.state.value === undefined) {
      entry.state.failure = null
    }
    const reading = (async () => {
      try {
        const value = await entry.spec.read(entry.state.value)
        entry.state.value = value
        entry.state.failure = null
        this.measure(entry, entry.spec.size(value))
        entry.confirmed = this.confirms(coverage, entry, stamp)
      } catch (error) {
        const failure = failureOf(error)
        entry.state.failure = failure
        // A path that is gone is an answer, as a first read's would be; any
        // other failure keeps what the view shows.
        if (failure.kind === 'not-found') {
          entry.state.value = undefined
          this.measure(entry, 0)
          entry.confirmed = this.confirms(coverage, entry, stamp)
        } else {
          entry.confirmed = false
        }
      } finally {
        entry.reading = null
        entry.state.reading = false
      }
      if (entry.stamp !== stamp && entry.shown > 0)
        this.reread(entry)
      this.evict()
    })()
    entry.reading = reading
    return reading
  }

  /**
   * Reads again an entry a view shows, for a report that named it: at once,
   * but a folder's listing at most once a second. A report within a second
   * of the listing's last read waits for that second to end and is then
   * read with any that came meanwhile, so the last report is never dropped.
   */
  private reread(entry: Entry<unknown>): void {
    if (entry.spec.kind !== 'listing') {
      void this.read(entry)
      return
    }
    if (entry.rereadTimer !== null)
      return
    const wait = entry.readStart + LISTING_REREAD_MS - performance.now()
    if (wait <= 0) {
      void this.read(entry)
      return
    }
    entry.rereadTimer = setTimeout(() => {
      entry.rereadTimer = null
      // A view that let go meanwhile reads it when it shows it next, as it is unconfirmed.
      if (entry.shown > 0)
        void this.read(entry)
    }, wait)
  }

  private cancelReread(entry: Entry<unknown>): void {
    if (entry.rereadTimer === null)
      return
    clearTimeout(entry.rereadTimer)
    entry.rereadTimer = null
  }

  private confirms(coverage: Coverage | null, entry: Entry<unknown>, stamp: number): boolean {
    return coverage !== null && this.coverages.has(coverage) && entry.stamp === stamp
  }

  private measure(entry: Entry<unknown>, size: number): void {
    this.kept += size - entry.size
    entry.size = size
  }

  /** Lets go of the entries shown longest ago, none shown now or being read, until the text kept fits the budget. */
  private evict(): void {
    if (this.kept <= this.budget)
      return
    const idle = [...this.entries.entries()]
      .filter(([, entry]) => entry.shown === 0 && !entry.reading)
      .sort(([, a], [, b]) => a.lastShown - b.lastShown)
    for (const [key, entry] of idle) {
      if (this.kept <= this.budget)
        return
      this.kept -= entry.size
      this.cancelReread(entry)
      this.entries.delete(key)
    }
  }

  private unconfirm(entry: Entry<unknown>): void {
    entry.confirmed = false
    entry.stamp += 1
  }

  /**
   * A view shows the entry `spec` names until it releases it: what is kept
   * shows at once, and an entry not kept or not confirmed is read.
   */
  show<T>(spec: KeptSpec<T>): Showing<T> {
    const entry = this.entry(spec)
    entry.shown += 1
    entry.lastShown = ++this.clock
    if (!entry.confirmed)
      void this.read(entry)
    let released = false
    return {
      entry: entry.state,
      retry: () => {
        void this.read(entry)
      },
      release: () => {
        if (released)
          return
        released = true
        entry.shown -= 1
        entry.lastShown = ++this.clock
        this.evict()
      },
    }
  }

  /**
   * Edits what is kept of the entry `spec` names, as the page's own write
   * tells it, without a read: an upload's new file in its folder's listing.
   * Nothing kept, nothing to edit; whether it is confirmed stays as it was,
   * since the Host's watch reports the write too.
   */
  amend<T>(spec: KeptSpec<T>, edit: (value: T) => T): void {
    const entry = this.entry(spec)
    const { value } = entry.state
    if (value === undefined)
      return
    const edited = edit(value)
    entry.state.value = edited
    this.measure(entry, spec.size(edited))
  }

  /** Reads the entry `spec` names again now, as a Refresh does. */
  retry<T>(spec: KeptSpec<T>): void {
    void this.read(this.entry(spec))
  }

  /** What is kept of the entry `spec` names, as a view would show it, without showing or reading it. */
  peek<T>(spec: KeptSpec<T>): ShownEntry<T> {
    return this.entry(spec).state
  }

  /** The entry once, for work that shows nothing: the confirmed answer kept, or a read's. */
  async get<T>(spec: KeptSpec<T>): Promise<T> {
    const entry = this.entry(spec)
    entry.lastShown = ++this.clock
    if (!entry.confirmed)
      await this.read(entry)
    const { value, failure } = entry.state
    if (value === undefined)
      throw new FileBrowserError(failure?.kind ?? 'other', failure?.message)
    return value
  }

  /**
   * Something changed at each of `paths`, of which git ignores `ignored`:
   * every entry they concern is unconfirmed, and read again while a view
   * shows it, at once but a listing at most once a second. An ignored path, such as a log a process appends to,
   * concerns its file and folder but not the working tree's changes
   * (`web-api.md` § File text and working tree changes).
   */
  changed(paths: readonly string[], ignored: readonly string[] = []): void {
    const ignoredPaths = new Set(ignored)
    for (const entry of this.entries.values()) {
      if (!paths.some((path) => entry.concerns(path, ignoredPaths.has(path))))
        continue
      this.unconfirm(entry)
      if (entry.shown > 0)
        this.reread(entry)
    }
  }

  /**
   * A watch is live: what it covers, read from now on, is confirmed. Each
   * entry it covers that a view shows and no live watch confirmed is read
   * again now: it may have changed unreported, or its last read failed while
   * the Host was away, which a watch going live says it is no longer.
   */
  cover(coverage: Coverage): void {
    this.coverages.add(coverage)
    for (const entry of this.entries.values()) {
      if (entry.shown > 0 && !entry.confirmed && coverage.covers(entry.spec.path))
        void this.read(entry)
    }
  }

  /**
   * A watch stopped covering: it ended, or lost reports. What it covered
   * may have changed unreported, so it is unconfirmed; with `reread`, as
   * after lost reports on a watch that goes on, the entries shown are read
   * again at once.
   */
  uncover(coverage: Coverage, reread = false): void {
    if (!this.coverages.delete(coverage))
      return
    for (const entry of this.entries.values()) {
      if (!coverage.covers(entry.spec.path))
        continue
      this.unconfirm(entry)
      if (reread && entry.shown > 0)
        void this.read(entry)
    }
  }

  /** Reads every shown entry `where` picks again now, as a Refresh does. */
  refresh(where: (path: string) => boolean = () => true): void {
    for (const entry of this.entries.values()) {
      if (entry.shown > 0 && where(entry.spec.path))
        void this.read(entry)
    }
  }
}
