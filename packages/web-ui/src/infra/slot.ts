import { computed, shallowRef, type ComputedRef, type ShallowRef } from 'vue'
import type { SerialQueue } from '@demicodes/utils'

/**
 * What a write's answer says of the slot (`web-application.md` § Responding
 * to the user).
 */
export interface SlotAnswer<V> {
  /**
   * The value the answer leaves, made of the value read. None when the
   * answer does not tell it: the change then waits for the slot's next value.
   */
  value?: (read: V) => V
  /** The slot's revision once the write is in it, when the backend numbers its values. */
  revision?: number
}

/** How a write ended for the change that stood for it. */
export type SlotWrite = 'landed' | 'waiting' | 'refused'

/**
 * One change of the page's that its write has not landed yet: shown over the
 * slot's value from the moment the user made it.
 */
export class SlotChange<V> {
  /** Where the slot's clock stood when the write went out; null before. */
  sentAt: number | null = null
  /**
   * Set once the write landed without its answer being applied: the change
   * stays until a value of the slot that is newer than its write arrives.
   */
  waiting: { landedAt: number; until: number | undefined } | null = null

  constructor(
    private readonly slot: Slot<V>,
    readonly part: string,
    readonly apply: (value: V) => V,
  ) {}

  /** Its write goes out now; a value of the slot read from here may be newer than the write's. */
  send(): void {
    this.slot.send([this])
  }

  /**
   * The write was answered: `answer`, or, without one, the change itself,
   * applies as the slot's value unless a value of the slot was read since the
   * write went out, and the change leaves. Otherwise it stays until the
   * slot's next value. Answers whether it left.
   */
  land(answer?: SlotAnswer<V>): boolean {
    return this.slot.land([this], answer)
  }

  /**
   * The write failed or the backend refused it: the change leaves at once,
   * and `answer`, what the backend has, applies as in `land`.
   */
  drop(answer?: SlotAnswer<V>): void {
    this.slot.drop([this], answer)
  }
}

/**
 * A piece of state the page reads from the backend (`web-application.md`
 * § Responding to the user): the value it last read, the revision that value
 * carries, and the changes the page made and has not seen land. The page
 * shows `value`, the value read with those changes applied in the order they
 * were made. One mechanism serves every slot:
 *
 * - A change leaves when its answer is applied, or, when a value of the slot
 *   was read between its write and its answer, with the next value read
 *   after the answer (or, when the answer names a revision, with the value
 *   of that revision).
 * - A change whose write failed or was refused leaves at once; the page then
 *   shows what the backend has.
 * - The writes of one thing go one at a time, in the order the user made
 *   them (`write` with a `SerialQueue`), those of different things side by
 *   side.
 *
 * A slot may be read in parts, as the product state is: a `part` names the
 * piece of the value a read or an answer replaces, and only reads and answers
 * of the same part order against a change.
 */
export class Slot<V> {
  /** The value as the backend last told it; null before the first read. */
  readonly read: ShallowRef<V | null> = shallowRef(null)
  /** The changes not landed yet, in the order they were made. */
  private readonly changes = shallowRef<SlotChange<V>[]>([])
  private readonly held = shallowRef(-1)
  private clock = 0
  /** The clock at each part's last read, and at the last read of the whole. */
  private readonly partAt = new Map<string, number>()
  private wholeAt = 0
  /** The newest revision a read was started for. */
  private asked = -1
  /** The value the page shows: the value read, or `empty` before the first read, with its changes on top. */
  readonly value: ComputedRef<V | null>

  /** `empty` is the value to show before the first read, for a slot whose first read is a list that is nothing yet. */
  constructor(private readonly empty: V | null = null) {
    this.value = computed(() => {
      const base = this.read.value ?? this.empty
      return base === null ? null : this.changes.value.reduce<V>((value, change) => change.apply(value), base)
    })
  }

  /** The revision of the value last read; -1 before the first read. */
  get revision(): number {
    return this.held.value
  }

  /** The slot's clock, which numbers its reads: a write sent at this point is measured against it. */
  now(): number {
    return this.clock
  }

  /**
   * Takes a read of the whole value, as a snapshot or a request answers. A
   * read of a revision no newer than the one held changes nothing. `since` is
   * where the clock stood when the request for it went out; a read from the
   * channel needs none. Answers whether it was taken.
   */
  replace(value: V, options: { revision?: number; since?: number } = {}): boolean {
    const { revision } = options
    if (revision !== undefined) {
      if (this.read.value !== null && revision <= this.held.value) {
        return false
      }
      this.held.value = revision
    }
    const at = ++this.clock
    this.wholeAt = at
    this.read.value = value
    this.releaseWaiting(() => true, options.since ?? at)
    return true
  }

  /**
   * Takes a read of one part, as a message of the channel brings it: `update`
   * makes the value of the one it replaces. Nothing is read before the whole.
   */
  receive(part: string, update: (read: V) => V): boolean {
    const read = this.read.value
    if (read === null) {
      return false
    }
    const at = ++this.clock
    this.partAt.set(part, at)
    this.read.value = update(read)
    this.releaseWaiting((change) => change.part === part, at)
    return true
  }

  /**
   * Applies what a write's answer says of `part`, as `answer` makes of the
   * value read, unless the slot was read since the write went out `at`:
   * that value may be newer, and if it is older, the value the write caused
   * is still to come. Answers whether it applied.
   */
  answered(part: string, at: number, answer: SlotAnswer<V> & { value: (read: V) => V }): boolean {
    const read = this.read.value
    if (read === null || Math.max(this.wholeAt, this.partAt.get(part) ?? 0) > at) {
      return false
    }
    this.read.value = answer.value(read)
    if (answer.revision !== undefined) {
      this.held.value = Math.max(this.held.value, answer.revision)
    }
    // The answer is newer than every write of the part that landed before it.
    this.releaseWaiting((change) => change.part === part, Infinity)
    return true
  }

  /**
   * Shows `apply`, a change the user made of `part`, over the value at once,
   * until its write lands. The caller sends the write, after the writes
   * before it, and says how it ended through the change.
   */
  change(apply: (value: V) => V, part = ''): SlotChange<V> {
    const change = new SlotChange(this, part, apply)
    this.changes.value = [...this.changes.value, change]
    return change
  }

  /**
   * Makes the write of `changes` and says how it ended: `request` goes out in
   * its turn among the writes of `keys` in `queue`, and answers what the
   * write says of the slot, or nothing. A write that fails is a refusal: the
   * changes leave and `refused` reports it.
   */
  write(
    changes: SlotChange<V> | readonly SlotChange<V>[],
    queue: SerialQueue,
    keys: string | readonly string[],
    request: () => Promise<SlotAnswer<V> | void>,
    refused: (error: unknown) => void,
  ): Promise<SlotWrite> {
    const sent = Array.isArray(changes) ? changes : [changes as SlotChange<V>]
    return queue.run(keys, async (): Promise<SlotWrite> => {
      try {
        this.send(sent)
        const answer = await request()
        return this.land(sent, answer ?? undefined) ? 'landed' : 'waiting'
      } catch (error) {
        this.drop(sent)
        refused(error)
        return 'refused'
      }
    })
  }

  /** The writes of `changes` go out now, together. */
  send(changes: readonly SlotChange<V>[]): void {
    for (const change of changes) {
      change.sentAt = this.clock
    }
  }

  /**
   * The write of `changes`, which went out together, was answered
   * (`SlotChange.land`); without an answer, the changes applied in order are
   * the value. Answers whether the changes left.
   */
  land(changes: readonly SlotChange<V>[], answer?: SlotAnswer<V>): boolean {
    // A change released since, as by a sign-out, has nothing left to say,
    // and one that landed waits only for the slot's next value.
    const live = changes.filter((change) => this.changes.value.includes(change) && change.waiting === null)
    const first = live[0]
    if (!first) {
      return true
    }
    const said = answer ?? { value: (read: V) => live.reduce((value, change) => change.apply(value), read) }
    const at = Math.min(...live.map((change) => change.sentAt ?? this.clock))
    if (said.value !== undefined && this.answered(first.part, at, { ...said, value: said.value })) {
      this.leave(live)
      return true
    }
    if (said.revision !== undefined && said.revision <= this.held.value) {
      // The slot holds a value at least as new as the answer's.
      this.leave(live)
      return true
    }
    const landedAt = ++this.clock
    for (const change of live) {
      change.waiting = { landedAt, until: said.revision }
    }
    return false
  }

  /** The write of `changes` failed or was refused (`SlotChange.drop`). */
  drop(changes: readonly SlotChange<V>[], answer?: SlotAnswer<V>): void {
    const live = changes.filter((change) => this.changes.value.includes(change) && change.waiting === null)
    const first = live[0]
    if (!first) {
      return
    }
    if (answer?.value !== undefined) {
      const at = Math.min(...live.map((change) => change.sentAt ?? this.clock))
      this.answered(first.part, at, { ...answer, value: answer.value })
    }
    this.leave(live)
  }

  /**
   * Whether a read of revision `revision` is worth starting: it is newer than
   * the value held and than any read started before, which it then counts as.
   */
  ask(revision: number): boolean {
    if (revision <= Math.max(this.held.value, this.asked)) {
      return false
    }
    this.asked = revision
    return true
  }

  /** Forgets everything, as a page that signed out does. */
  clear(): void {
    this.read.value = null
    this.changes.value = []
    this.held.value = -1
    this.clock = 0
    this.partAt.clear()
    this.wholeAt = 0
    this.asked = -1
  }

  private leave(left: readonly SlotChange<V>[]): void {
    this.changes.value = this.changes.value.filter((change) => !left.includes(change))
  }

  /**
   * The waiting changes `of` names leave when the value read now is newer
   * than their write: read after it landed, or, when the answer named a
   * revision, at that revision. `since` is where the clock stood when the
   * request for the read went out.
   */
  private releaseWaiting(of: (change: SlotChange<V>) => boolean, since: number): void {
    const released = (change: SlotChange<V>) => {
      const { waiting } = change
      if (waiting === null || !of(change)) {
        return false
      }
      return waiting.until === undefined ? since >= waiting.landedAt : this.held.value >= waiting.until
    }
    if (this.changes.value.some(released)) {
      this.changes.value = this.changes.value.filter((change) => !released(change))
    }
  }
}
